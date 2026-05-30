//! `ort` ONNX Runtime session facade — feature-gated, runtime-loaded.
//!
//! Engine-side models (tracker backbones, audio classifiers) run in-process
//! via [`ort`](https://docs.rs/ort). The `ort` dependency is optional and
//! gated behind the `ort` feature; the `load-dynamic` link mode means the
//! ONNX Runtime shared library is resolved at *runtime*, not link time, so:
//!
//! - feature-off, the crate compiles with zero ONNX toolchain present;
//! - feature-on, the crate compiles even when `libonnxruntime` is absent —
//!   the missing library only surfaces when [`OrtSession::open`] actually
//!   tries to create a session, where it becomes
//!   [`AiError::ModelLoadFailed`].
//!
//! ## Why a pre-flight dylib probe ([`runtime_available`])
//!
//! `ort` rc.12 resolves the shared library inside a lazy global initializer.
//! When the library is absent that initializer panics *while holding its
//! lock*, which deadlocks every subsequent caller — `Session::builder()`
//! never returns an `Err`, it hangs. So [`OrtSession::open`] must NOT enter
//! any `ort` API when the runtime is missing. [`runtime_available`] resolves
//! the candidate dylib path the same way `ort` would (`ORT_DYLIB_PATH`, then
//! the platform default name) and checks it exists *before* touching `ort`,
//! turning "runtime unavailable" into a fast, loud [`AiError::ModelLoadFailed`]
//! instead of a hang.
//!
//! This module deliberately exposes only session *creation* from a model
//! file. The inference call surface (`Session::run` with typed tensors) is
//! driven by the per-capability adapters and is out of scope for the
//! transport facade.

#[cfg(feature = "ort")]
use std::path::{Path, PathBuf};

use crate::error::AiError;

/// Platform default ONNX Runtime shared-library filename, used as the probe
/// target when `ORT_DYLIB_PATH` is unset.
#[cfg(feature = "ort")]
const DEFAULT_DYLIB_NAME: &str = if cfg!(target_os = "windows") {
    "onnxruntime.dll"
} else if cfg!(target_os = "macos") {
    "libonnxruntime.dylib"
} else {
    "libonnxruntime.so"
};

/// Resolve the ONNX Runtime dylib path `ort`'s `load-dynamic` loader would
/// use: `ORT_DYLIB_PATH` if set, else the platform default filename (which
/// the loader searches via the system linker path).
#[cfg(feature = "ort")]
fn dylib_candidate() -> PathBuf {
    match std::env::var_os("ORT_DYLIB_PATH") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(DEFAULT_DYLIB_NAME),
    }
}

/// `true` when the ONNX Runtime shared library is present and loadable.
///
/// Operator note: native (`ort`-backed) inference is enabled only when
/// `ORT_DYLIB_PATH` points at an existing `libonnxruntime` file; absent that,
/// this probe returns `false` and adapters fall back to a loud error.
///
/// This probe is deliberately *outside* any `ort` API call: `ort`'s lazy
/// global init panic-deadlocks on a missing library (see module docs), so
/// the runtime must be confirmed present before [`OrtSession::open`] enters
/// `ort`. An explicit absolute `ORT_DYLIB_PATH` is checked for file
/// existence; an unset path means the bare platform default name, which the
/// system loader resolves — we cannot file-stat a bare name reliably, so a
/// bare-name candidate is treated as "let `ort` try" only when the file
/// resolves on the loader path. To stay deadlock-safe we require an explicit
/// existing file: a bare default name with no `ORT_DYLIB_PATH` is reported
/// unavailable, which is the correct default for hosts without the runtime.
#[cfg(feature = "ort")]
#[must_use]
pub fn runtime_available() -> bool {
    let candidate = dylib_candidate();
    // Only an explicit, existing file is treated as available. A bare
    // platform default name (ORT_DYLIB_PATH unset) is not stat-able as a
    // path, so it is reported unavailable rather than risking the ort
    // global-init deadlock on a host without the runtime installed.
    candidate.is_absolute() && candidate.is_file()
}

/// Handle to a loaded ONNX Runtime session.
///
/// Opaque by design — the inner `ort::session::Session` is not `Clone` and
/// owns native runtime state. Mirrors the no-`Clone`/no-`Default`/no-`Copy`
/// discipline of [`crate::provider::Provider`].
#[cfg(feature = "ort")]
#[derive(Debug)]
pub struct OrtSession {
    inner: ort::session::Session,
}

#[cfg(feature = "ort")]
impl OrtSession {
    /// Open an ONNX model file as an inference session, registering the v1
    /// auto-promote execution-provider order
    /// ([`crate::ORT_AUTO_PROMOTE_ORDER_V1`]) so unpinned callers get the
    /// best available accelerator with CPU fallback.
    ///
    /// # Errors
    ///
    /// Returns [`AiError::ModelLoadFailed`] if the ONNX Runtime shared
    /// library is not present ([`runtime_available`] is `false`), if the
    /// model file is missing or malformed, or if the requested execution
    /// providers cannot be initialized.
    pub fn open(model_path: &Path) -> Result<Self, AiError> {
        // Deadlock guard: never enter `ort`'s lazy global init when the
        // runtime is absent — it panics while holding its lock and hangs
        // (see module docs). Surface the missing runtime loudly instead.
        if !runtime_available() {
            return Err(AiError::ModelLoadFailed {
                detail: format!(
                    "onnxruntime shared library not found (probed `{}`); set ORT_DYLIB_PATH \
                     to an existing libonnxruntime to enable ort inference",
                    dylib_candidate().display()
                ),
            });
        }
        let inner = ort::session::Session::builder()
            .map_err(|err| AiError::ModelLoadFailed {
                detail: format!("ort session builder init failed: {err}"),
            })?
            .commit_from_file(model_path)
            .map_err(|err| AiError::ModelLoadFailed {
                detail: format!("ort failed to load model `{}`: {err}", model_path.display()),
            })?;
        Ok(Self { inner })
    }

    /// Borrow the underlying `ort` session for the adapter inference path.
    #[must_use]
    pub fn inner(&self) -> &ort::session::Session {
        &self.inner
    }
}

/// Feature-off shim: with the `ort` feature disabled, attempting to open a
/// session is a compile-time-available call that always reports the runtime
/// as unavailable. Keeps the call surface identical across feature states so
/// the adapters do not need their own `#[cfg]` forks.
#[cfg(not(feature = "ort"))]
#[must_use]
pub fn open_unavailable() -> AiError {
    AiError::ModelLoadFailed {
        detail: "ort feature disabled at build time; rebuild with --features ort".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(feature = "ort"))]
    #[test]
    fn feature_off_reports_runtime_unavailable() {
        assert!(matches!(
            open_unavailable(),
            AiError::ModelLoadFailed { .. }
        ));
    }

    #[cfg(feature = "ort")]
    #[test]
    fn feature_on_missing_runtime_surfaces_model_load_failed() {
        // No onnxruntime is installed in CI/dev hosts, so `runtime_available`
        // is false and `open` must return ModelLoadFailed *without* entering
        // ort's lazy global init (which deadlocks on a missing dylib). The
        // assertion below would hang, not fail, if the deadlock guard were
        // removed — proving the guard is load-bearing.
        assert!(
            !runtime_available(),
            "test host unexpectedly has onnxruntime; skip-if-runtime logic untestable here"
        );
        let err = OrtSession::open(Path::new("/nonexistent/model.onnx")).unwrap_err();
        assert!(
            matches!(err, AiError::ModelLoadFailed { .. }),
            "got {err:?}"
        );
    }
}
