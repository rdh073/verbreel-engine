//! verbreel-ai — ONNX Runtime + Python sidecar dispatch.
//!
//! Two inference transports land here (Research 04 §3 / §9.3):
//!
//! - **Python sidecar** ([`sidecar`]) — JSON-over-stdin/stdout to a
//!   spawned child (faster-whisper STT, `BeatNet`). This transport is
//!   exercised end-to-end against a real `python3` child in the crate
//!   tests: a request is written, the child responds, and the response
//!   is decoded into the state-owned canonical struct. The model science
//!   lives in the (operator-installed) sidecar script, not in this crate.
//! - **`ort` session** ([`session`], feature `ort`) — in-process ONNX
//!   Runtime. The session-open call is a runtime gate that fails loud
//!   ([`AiError::ModelLoadFailed`]) when `libonnxruntime` is absent.
//!   Folding model output into a result needs shipped `.onnx` weights,
//!   which are NOT part of this slice — so the `ort` adapter arm is a
//!   provably loud error path today, never a fabricated result (Linus
//!   rule). It returns a result only once the weight assets ship.
//!
//! Note `onset` / `tempo` audio analysis are spec'd as **native Rust
//! DSP** (Research 04 §6.1), not a transport this crate spawns; the
//! registry advertises them as engine-native entries.
//!
//! ## Crate dependency rule
//!
//! ```text
//! verbreel-ai → verbreel-types, verbreel-state
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]

pub mod adapters;
pub mod capability;
pub mod error;
pub mod execution_provider;
pub mod model;
pub mod provider;
pub mod registry;
pub mod session;
pub mod sidecar;

pub use adapters::{Backend, run_audio_analysis, run_stt, run_tracker};
pub use capability::Capability;
pub use error::AiError;
pub use execution_provider::{
    ExecutionProvider, ORT_AUTO_PROMOTE_ORDER_V1, ORT_AUTO_PROMOTE_ORDER_V1_IDS,
    ort_auto_promote_order_v1,
};
pub use model::{ModelCacheKey, ModelId, ModelVersion};
pub use provider::{Provider, run};
pub use registry::{ProviderEntry, ProviderRegistry};
pub use sidecar::{SidecarRequest, SidecarResponse, run_sidecar};

#[cfg(feature = "ort")]
pub use session::OrtSession;
