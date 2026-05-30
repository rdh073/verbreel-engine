//! Browser smoke harness for the wasm32 preview-session bridge.
//!
//! Gated `#[cfg(target_arch = "wasm32")]` so it compiles only for the
//! browser target and never runs in the native CI matrix. Run it with a
//! headless browser:
//!
//! ```sh
//! wasm-pack test --headless --chrome -p verbreel-wasm
//! ```
//!
//! These exercise the bridge end to end against a live `VideoDecoder`:
//! instantiate an [`EngineHandle`], open a [`PreviewSession`] from probed
//! capabilities, drive `seek` / `frameAt`, and confirm the frame-handle
//! plumbing. A full pixel-accurate decode needs a real encoded chunk
//! fixture, which the cross-vendor pixel-diff harness (#17) owns; here
//! the control path is asserted with a one-byte chunk so `frameAt`
//! returns no frame rather than asserting decoded pixels.

#![cfg(target_arch = "wasm32")]

use verbreel_codec_web::capability;
use verbreel_wasm::{EngineHandle, init_diagnostics};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// Whether the live browser exposes a `WebCodecs` `VideoDecoder`. Tests
/// that need a real decoder skip cleanly where it is absent so the suite
/// passes on any engine.
fn has_webcodecs() -> bool {
    capability::detect().has_webcodecs_decode
}

#[wasm_bindgen_test]
fn diagnostics_init_is_idempotent() {
    // The panic hook + tracing bridge install must be safe to call more
    // than once (try_set_as_global_default / set_once).
    init_diagnostics();
    init_diagnostics();
}

#[wasm_bindgen_test]
fn handle_reports_preview_only_scope() {
    let handle = EngineHandle::new();
    assert!(handle.supports_preview_embedding());
    assert!(!handle.supports_editor_embedding());
    assert_eq!(handle.embedding_scope_wire(), "preview-only");
}

#[wasm_bindgen_test]
fn open_preview_session_resolves_transport_from_caps() {
    // The bridge resolves codec-web's transport policy: webcodecs when
    // the browser has VideoDecoder, mse otherwise — matching the live
    // probe.
    let handle = EngineHandle::new();
    let caps = capability::detect();
    let session = handle
        .open_preview_session(caps.has_webcodecs_decode, false, false)
        .expect("opening a preview session must succeed");
    if caps.has_webcodecs_decode {
        assert_eq!(session.transport_literal(), "webcodecs");
    } else {
        assert_eq!(session.transport_literal(), "mse");
    }
    assert_eq!(session.codec_literal(), "h264");
}

#[wasm_bindgen_test]
fn seek_positions_session_in_micros() {
    let handle = EngineHandle::new();
    let mut session = handle
        .open_preview_session(has_webcodecs(), false, false)
        .expect("open ok");
    session.seek(240_000); // 1 s at 240 kHz
    assert_eq!(session.seek_micros(), 1_000_000);
}

#[wasm_bindgen_test]
async fn frame_at_pulls_one_frame_handle_against_live_decoder() {
    // The required smoke path: instantiate handle, open a preview
    // session against the codec-web decoder, pull a frame handle. With a
    // one-byte stub chunk the decoder produces no frame, so `frameAt`
    // returns `None` — the assertion is that the full open → seek →
    // frameAt plumbing runs and yields the Option<PreviewFrame> handle
    // type, not that a specific frame decodes (that belongs to #17).
    if !has_webcodecs() {
        return;
    }
    let handle = EngineHandle::new();
    let mut session = handle
        .open_preview_session(true, false, false)
        .expect("open ok where VideoDecoder present");
    session.seek(0);
    let frame = session
        .frame_at(&[0u8], true)
        .await
        .expect("frameAt control path must not error on a stub chunk");
    assert!(
        frame.is_none(),
        "a single stub byte decodes to no frame; got a frame handle"
    );
}
