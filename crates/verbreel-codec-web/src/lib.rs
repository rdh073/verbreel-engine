//! verbreel-codec-web — `WebCodecs` shim (wasm32-only).
//!
//! First content slice: v0 decode-only type surface committing
//! Research 01 candidates §1's "decode-only, preview-only — no encode
//! in browser" decision to types. The implementation body in
//! [`decode`] returns [`DecodeError::NotYetImplemented`] until Spike
//! S2 lands the real `web_sys::VideoDecoder` path and the shared-WGSL
//! native↔wasm32 pixel-diff harness.
//!
//! Modules ([`SoC`](https://en.wikipedia.org/wiki/Separation_of_concerns)
//! — one concern per file):
//!
//! - [`decoder`] — [`WebDecoder`] enum (`H264`, `H265`). AV1 / VP9 and
//!   other browser-mediated codecs deferred until `WebCodecs` decoder
//!   enumeration confirms cross-browser parity (Research 01 §R3).
//! - [`error`] — [`DecodeError`] enum with the four v0 failure modes.
//! - [`frame`] — opaque [`DecodedFrame`] placeholder so [`decode`] has
//!   a complete signature (mirrors codec-native's [`Frame`] but adds
//!   `pts_micros` to match the `WebCodecs` `VideoFrame.timestamp` unit).
//! - [`decode`](self::decode::decode) — v0 entry point returning
//!   [`DecodeError::NotYetImplemented`].
//! - [`preview_codec`] — `preview.session` transport decision surface
//!   (`webcodecs` primary, fMP4/MSE fallback).
//!
//! Research 01 references:
//!
//! - candidates §1 — decode-only `WebCodecs` path; no encode in browser.
//! - Risks §R3 — Safari `WebCodecs` parity gate on the codec set.
//! - §11 — Spike S2 shared-WGSL native↔wasm32 pixel-diff acceptance
//!   criterion (≤1/255 per channel).
//!
//! [`Frame`]: ../verbreel_codec_native/struct.Frame.html

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod decode;
pub mod decoder;
pub mod error;
pub mod frame;
pub mod preview_codec;

pub use decode::decode;
pub use decoder::WebDecoder;
pub use error::DecodeError;
pub use frame::DecodedFrame;
pub use preview_codec::{
    BrowserFamily, PREVIEW_CODEC_MSE, PREVIEW_CODEC_WEBCODECS, PreviewClientCapabilities,
    WebPreviewCodec, codec_for_preview, safari_fallback_codec,
};
