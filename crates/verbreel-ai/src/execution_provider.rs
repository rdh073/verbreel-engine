//! `ort` execution-provider policy surface for v1.
//!
//! This module defines only the policy ordering for unpinned
//! engine-side `ort` calls. It does not probe host capabilities,
//! load sessions, or implement runtime fallback loops.

/// ONNX Runtime execution provider kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionProvider {
    /// NVIDIA CUDA execution provider.
    Cuda,
    /// NVIDIA `TensorRT` execution provider.
    TensorRt,
    /// Windows `DirectML` execution provider.
    DirectMl,
    /// Apple `CoreML` execution provider.
    CoreMl,
    /// CPU execution provider.
    Cpu,
}

impl ExecutionProvider {
    /// Canonical lowercase id used by config arrays.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::TensorRt => "tensorrt",
            Self::DirectMl => "directml",
            Self::CoreMl => "coreml",
            Self::Cpu => "cpu",
        }
    }
}

/// v1 policy for unpinned engine-side `ort` execution providers:
/// CUDA -> `TensorRT` -> `DirectML` -> `CoreML` -> CPU.
pub const ORT_AUTO_PROMOTE_ORDER_V1: [ExecutionProvider; 5] = [
    ExecutionProvider::Cuda,
    ExecutionProvider::TensorRt,
    ExecutionProvider::DirectMl,
    ExecutionProvider::CoreMl,
    ExecutionProvider::Cpu,
];

/// Canonical ids for [`ORT_AUTO_PROMOTE_ORDER_V1`].
pub const ORT_AUTO_PROMOTE_ORDER_V1_IDS: [&str; 5] =
    ["cuda", "tensorrt", "directml", "coreml", "cpu"];

/// Returns the v1 `ort` auto-promote order for unpinned execution
/// provider selection.
#[must_use]
pub fn ort_auto_promote_order_v1() -> &'static [ExecutionProvider] {
    &ORT_AUTO_PROMOTE_ORDER_V1
}
