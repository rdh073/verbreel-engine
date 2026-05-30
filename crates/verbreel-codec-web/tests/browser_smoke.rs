//! Browser smoke tests for the wasm32 `WebCodecs` path.
//!
//! Gated `#[cfg(target_arch = "wasm32")]` so they compile only for the
//! browser target and never run in the native CI matrix. Run them with
//! a headless browser:
//!
//! ```sh
//! wasm-pack test --headless --chrome -p verbreel-codec-web
//! ```
//!
//! These exercise the capability probe and the handshake against a live
//! browser; the full decode loop needs a real encoded chunk fixture,
//! which the cross-vendor pixel-diff harness (#17) owns and is out of
//! scope here.

#![cfg(target_arch = "wasm32")]

use verbreel_codec_web::capability;
use verbreel_codec_web::{
    PreviewSessionPlan, PreviewSessionTransport, WebCodecsSession, WebDecoder,
};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

#[wasm_bindgen_test]
fn capability_probe_drives_handshake() {
    // The live browser reports its own capabilities; the handshake must
    // produce a transport whose literal matches the probed support.
    let caps = capability::detect();
    let plan = PreviewSessionPlan::resolve(caps, WebDecoder::H264);
    if caps.has_webcodecs_decode {
        assert_eq!(plan.codec_literal(), "webcodecs");
        assert_eq!(plan.transport, PreviewSessionTransport::WebCodecs);
    } else {
        assert_eq!(plan.codec_literal(), "mse");
        assert!(matches!(
            plan.transport,
            PreviewSessionTransport::Mse { .. }
        ));
    }
}

#[wasm_bindgen_test]
fn webcodecs_session_configures_in_browser() {
    // Only meaningful where the browser actually exposes VideoDecoder;
    // skip cleanly elsewhere so the suite passes on any engine.
    if !capability::detect().has_webcodecs_decode {
        return;
    }
    let session = WebCodecsSession::new().expect("VideoDecoder must construct when present");
    session
        .configure(WebDecoder::H264)
        .expect("H.264 config must be accepted");
}

#[wasm_bindgen_test]
async fn flush_then_drain_returns_empty_for_no_input() {
    // Exercises the full submit/flush/drain plumbing against a live
    // decoder. With no chunks submitted, flush resolves and drain
    // returns no frames — the decode loop's control path, asserted
    // without a codec fixture (those belong to the pixel-diff harness,
    // #17).
    if !capability::detect().has_webcodecs_decode {
        return;
    }
    let session = WebCodecsSession::new().expect("VideoDecoder must construct when present");
    session
        .configure(WebDecoder::H264)
        .expect("H.264 config must be accepted");
    session
        .flush()
        .await
        .expect("flush of empty queue resolves");
    let frames = session.drain().await.expect("drain of empty queue ok");
    assert!(frames.is_empty(), "no chunks submitted => no frames");
}
