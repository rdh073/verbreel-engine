//! verbreel-codec-web — browser preview decode (`WebCodecs` +
//! fMP4/MSE fallback).
//!
//! The crate decides the browser preview transport and, on `wasm32`,
//! drives the real `WebCodecs` decode path. Policy #405 option A is
//! pinned: use `webcodecs` when the client has `VideoDecoder` decode,
//! otherwise fall back to fMP4/MSE (`mse`), including Safari.
//!
//! ## Native vs wasm32 split
//!
//! Browser dependencies (`wasm-bindgen`, `js-sys`, `web-sys`) live
//! under `[target.'cfg(target_arch = "wasm32")'.dependencies]`, so a
//! native `cargo check --workspace` pulls no browser toolchain. The
//! native surface is the pure policy/handshake logic plus the no-op
//! [`decode`] entry point; the wasm32 build additionally compiles
//! [`webcodecs`], the live `VideoDecoder` path.
//!
//! Frame bytes never enter the event log: only preview-session
//! metadata ([`PreviewSessionPlan`], [`MseFallbackEnvelope`])
//! serializes (Research 01 §6.2).
//!
//! ## Modules ([`SoC`](https://en.wikipedia.org/wiki/Separation_of_concerns)
//! — one concern per file)
//!
//! - [`decoder`] — [`WebDecoder`] enum (`H264`, `H265`). AV1 / VP9 and
//!   other browser-mediated codecs deferred until `WebCodecs` decoder
//!   enumeration confirms cross-browser parity (Research 01 §R3).
//! - [`error`] — [`DecodeError`] enum with the four failure modes.
//! - [`frame`] — opaque [`DecodedFrame`] (mirrors codec-native's
//!   [`Frame`] but adds `pts_micros` to match the `WebCodecs`
//!   `VideoFrame.timestamp` unit).
//! - [`decode`](self::decode::decode) — native no-op entry point
//!   returning [`DecodeError::NotYetImplemented`]; the live decode path
//!   on wasm32 is [`webcodecs`].
//! - [`preview_codec`] — `preview.session` transport decision policy
//!   (`webcodecs` primary, fMP4/MSE fallback).
//! - [`capability`] — capability source: a `wasm32` browser probe
//!   ([`capability::detect`]) feeding the policy.
//! - [`fallback`] — [`MseFallbackEnvelope`], the fMP4/MSE session
//!   metadata.
//! - [`handshake`] — [`PreviewSessionPlan`], the capability-report →
//!   chosen-transport composition point.
//! - [`webcodecs`] (wasm32 only) — [`webcodecs::WebCodecsSession`], the
//!   live `web_sys::VideoDecoder` decode loop.
//!
//! The full transport-selection table lives in
//! [`docs/fallback-matrix.md`](https://github.com/rdh073/verbreel-engine/blob/main/crates/verbreel-codec-web/docs/fallback-matrix.md).
//!
//! Research 01 references:
//!
//! - candidates §1 — decode-only `WebCodecs` path; no encode in browser.
//! - Risks §R3 — Safari `WebCodecs` parity gate on the codec set.
//! - §6.2 — browser preview distinct from native canonical render;
//!   preview frames stay off the persisted timeline.
//!
//! [`Frame`]: ../verbreel_codec_native/struct.Frame.html

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod capability;
pub mod decode;
pub mod decoder;
pub mod error;
pub(crate) mod error_surface;
pub mod fallback;
pub mod frame;
pub mod handshake;
pub mod preview_codec;
#[cfg(target_arch = "wasm32")]
pub mod webcodecs;

pub use decode::decode;
pub use decoder::WebDecoder;
pub use error::DecodeError;
pub use fallback::MseFallbackEnvelope;
pub use frame::DecodedFrame;
pub use handshake::{PreviewSessionPlan, PreviewSessionTransport};
pub use preview_codec::{
    BrowserFamily, PREVIEW_CODEC_MSE, PREVIEW_CODEC_WEBCODECS, PreviewClientCapabilities,
    WebPreviewCodec, codec_for_preview, safari_fallback_codec,
};
#[cfg(target_arch = "wasm32")]
pub use webcodecs::WebCodecsSession;
