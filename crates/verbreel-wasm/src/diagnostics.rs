//! Developer diagnostics for the browser preview build.
//!
//! Installs two opt-in dev aids and nothing else — no telemetry, no
//! network, no analytics (Research 01: browser build ships preview, not
//! instrumentation):
//!
//! 1. `console_error_panic_hook` — routes Rust panics to
//!    `console.error` with a readable message and stack, instead of the
//!    opaque `RuntimeError: unreachable` a raw wasm panic produces.
//! 2. `wasm-tracing` — bridges the `tracing` macros the engine already
//!    emits onto `console.*` / `window.performance`, so a developer sees
//!    spans/events in devtools without wiring a subscriber by hand.
//!
//! Both are idempotent: `console_error_panic_hook::set_once` is a no-op
//! after the first call, and the tracing bridge uses
//! `try_set_as_global_default` so a second call (or a host that already
//! installed a subscriber) is a benign no-op rather than a panic.
//!
//! The entry point is a single `wasm-bindgen` export, [`init`], that JS
//! calls once at bundle start. It is target-gated: the install bodies
//! compile only on wasm32 (the panic-hook / tracing-bridge crates are
//! browser-only deps), and on native the function is a no-op so the
//! symbol and the `cargo check --workspace` build stay green.

use wasm_bindgen::prelude::wasm_bindgen;

/// Install browser dev diagnostics: the `console.error` panic hook and
/// the `tracing` → `console` bridge.
///
/// Call once from JS at bundle start. Idempotent — safe to call more
/// than once. No telemetry: this only wires Rust diagnostics to the
/// browser console for local development.
#[wasm_bindgen(js_name = initDiagnostics)]
pub fn init() {
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        // `try_set_*` (not `set_as_global_default`) so a repeat call or a
        // host-installed subscriber is a no-op, not a panic.
        let _ = wasm_tracing::try_set_as_global_default();
    }
}
