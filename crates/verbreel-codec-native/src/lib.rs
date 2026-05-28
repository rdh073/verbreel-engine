//! verbreel-codec-native — rsmpeg native decode/encode, hwaccel detection.
//!
//! First content slice: v0 type surface committing Research 01 §5 + §11's
//! architectural decisions to types. The implementation body in
//! [`encode`] returns [`CodecError::NotYetImplemented`] until Spike S1
//! lands the real rsmpeg+libx264 pipeline.
//!
//! Modules ([`SoC`](https://en.wikipedia.org/wiki/Separation_of_concerns)
//! — one concern per file):
//!
//! - [`codec`] — [`Codec`] enum (`H264`, `ProRes`). AV1/HEVC/hwaccel
//!   deferred to Research 01 §6 follow-up.
//! - [`preset`] — [`CodecPreset`] enum (`Deterministic`, `Performance`)
//!   pinning Research 01 §5 + §11.1 / §11.2's two-preset model.
//! - [`params`] — [`EncodeParams`] struct with the full encode-pass
//!   parameter shape.
//! - [`error`] — [`CodecError`] enum with the four v0 failure modes.
//! - [`frame`] — opaque [`Frame`] placeholder so [`encode`] has a
//!   complete signature.
//! - [`encode`](self::encode::encode) — v0 entry point returning
//!   [`CodecError::NotYetImplemented`].
//!
//! Research 01 references:
//!
//! - §5 — pluggable codec backends; libx264 software encoder as
//!   the deterministic-mode default.
//! - §11.1 — Spike S1 determinism pass criterion + canonical
//!   `-x264-params` string.
//! - §11.2 — Spike S1 performance pass criterion (≥60 fps, default
//!   libx264 threading).

#![deny(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::pedantic)]

pub mod codec;
pub mod encode;
pub mod error;
pub mod frame;
pub mod params;
pub mod preset;

pub use codec::Codec;
pub use encode::encode;
pub use error::CodecError;
pub use frame::Frame;
pub use params::EncodeParams;
pub use preset::CodecPreset;
