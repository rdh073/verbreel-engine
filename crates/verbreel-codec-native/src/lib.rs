//! verbreel-codec-native — rsmpeg native decode/encode/probe facade.
//!
//! The encode half of Spike S1: rsmpeg decode → packed-`yuv420p` [`Frame`]s
//! → libx264/encoder → MP4 bytes, with deterministic-mode byte stability
//! pinned per Research 01 §11. The rsmpeg path links system `FFmpeg` 6.x and
//! lives behind the `rsmpeg` cargo feature so the default workspace
//! `cargo check` stays toolchain-free (CI has no `FFmpeg`). Feature-off, the
//! full type surface still compiles: [`encode`](self::encode::encode) fails
//! closed at runtime with [`CodecError::FeatureDisabled`], while `decode` and
//! `probe` are gated out as a whole module (the `decode` module is
//! `#[cfg(feature = "rsmpeg")]`), so referencing them feature-off is a
//! compile error rather than a runtime return.
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
//! - [`probe`] — [`ProbeMetadata`] struct feeding `asset.probe` /
//!   `asset.import` (`FFmpeg`-free type; populated by `decode::probe`).
//! - [`error`] — [`CodecError`] enum with the codec failure modes.
//! - [`frame`] — [`Frame`] packed-`yuv420p` pixel buffer shared by the
//!   decode and encode facades.
//! - [`encode`](self::encode::encode) — `EncodeParams` + frames → MP4 bytes.
//! - `decode` — `rsmpeg` decode + probe facade (feature `rsmpeg`).
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
#[cfg(feature = "rsmpeg")]
pub mod decode;
pub mod encode;
pub mod error;
pub mod frame;
pub mod params;
pub mod preset;
pub mod probe;

pub use codec::Codec;
pub use encode::encode;
pub use error::CodecError;
pub use frame::Frame;
pub use params::EncodeParams;
pub use preset::CodecPreset;
pub use probe::ProbeMetadata;
