//! Capability detection feeding [`crate::codec_for_preview`].
//!
//! The transport decision in [`crate::preview_codec`] is a pure
//! function of [`PreviewClientCapabilities`]. This module is the
//! *source* of that input:
//!
//! - On any target, [`PreviewClientCapabilities`] can be built
//!   directly from known values (the path the preview-session
//!   handshake and tests use — a capability report arrives over the
//!   wire and is deserialized, not probed locally).
//! - On `wasm32`, [`detect`] probes the live browser: it checks for a
//!   `VideoDecoder` global and infers the browser family from the user
//!   agent, so a WASM build can self-report without a server round
//!   trip.
//!
//! Keeping detection separate from the policy decision is the
//! [`SoC`](https://en.wikipedia.org/wiki/Separation_of_concerns) split
//! that lets the policy be a pure `const fn` tested natively while the
//! browser-only probe stays behind `cfg(target_arch = "wasm32")`.

pub use crate::preview_codec::{BrowserFamily, PreviewClientCapabilities};

/// Probe the live browser for `WebCodecs` decode support and browser
/// family.
///
/// Returns capabilities suitable to feed [`crate::codec_for_preview`].
/// `WebCodecs` presence is detected by the existence of the
/// `VideoDecoder` constructor on the global scope; the Safari family is
/// inferred from the user-agent string (Safari/WebKit reports `Safari`
/// without `Chrome`/`Chromium`).
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn detect() -> PreviewClientCapabilities {
    PreviewClientCapabilities {
        browser_family: detect_browser_family(),
        has_webcodecs_decode: has_webcodecs_decode(),
    }
}

/// True when the global scope exposes a `VideoDecoder` constructor.
#[cfg(target_arch = "wasm32")]
#[must_use]
fn has_webcodecs_decode() -> bool {
    use wasm_bindgen::JsValue;
    use wasm_bindgen::prelude::wasm_bindgen;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(thread_local_v2, js_name = VideoDecoder)]
        static VIDEO_DECODER: JsValue;
    }

    !VIDEO_DECODER.with(JsValue::is_undefined)
}

/// Infer the browser family from the navigator user-agent string.
///
/// Safari/WebKit reports `Safari` in the UA without `Chrome` or
/// `Chromium`; Chromium-family browsers also carry `Safari` for legacy
/// reasons, so the absence of `Chrome`/`Chromium` is the discriminator.
#[cfg(target_arch = "wasm32")]
#[must_use]
fn detect_browser_family() -> BrowserFamily {
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();
    let is_safari = ua.contains("Safari") && !ua.contains("Chrome") && !ua.contains("Chromium");
    if is_safari {
        BrowserFamily::Safari
    } else {
        BrowserFamily::Other
    }
}
