//! Spike S1 slice A — proves rsmpeg decode↔encode determinism.
//!
//! Not production code; not in default build. Behind `--features spike-s1`.
//! Pulls in `rsmpeg 0.18+ffmpeg.8` + `libx264` with the §5 canonical
//! deterministic preset (`threads=1:sliced-threads=0:sync-lookahead=0:
//! rc-lookahead=0:bframes=0`) and a frozen MP4 `creation_time` so two
//! sequential encode passes of the same input yield byte-identical output.

pub mod decoder;
pub mod encoder;
pub mod synth;

pub use decoder::Decoder;
pub use encoder::Encoder;
pub use synth::generate_raw_yuv420p;
