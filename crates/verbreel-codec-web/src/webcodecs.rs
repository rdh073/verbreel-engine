//! wasm32 `WebCodecs` bindings — the real browser decode path.
//!
//! This module is compiled only for `wasm32` (`#[cfg(target_arch =
//! "wasm32")]` at the `mod` site in [`crate`]); on native targets the
//! crate exposes the no-op surface in [`crate::decode`] and the pure
//! capability / handshake logic, so `cargo check --workspace` pulls no
//! browser dependencies.
//!
//! The `WebCodecs` decode types (`VideoDecoder`, `EncodedVideoChunk`,
//! `VideoFrame`) are still unstable in `web-sys` 0.3, so the wasm32
//! build sets `--cfg=web_sys_unstable_apis` via `.cargo/config.toml`.
//!
//! ## Shape
//!
//! [`WebCodecsSession`] owns a `web_sys::VideoDecoder` plus a shared
//! queue the decoder's `output` callback drains into. The flow mirrors
//! the `WebCodecs` contract:
//!
//! 1. [`WebCodecsSession::new`] wires the `output` / `error` callbacks.
//! 2. [`WebCodecsSession::configure`] sets the codec string.
//! 3. [`WebCodecsSession::decode_chunk`] submits one
//!    `EncodedVideoChunk`.
//! 4. The browser invokes the `output` callback per decoded
//!    `VideoFrame` *on a later task*; the callback retains the frame
//!    handle on the queue.
//! 5. [`WebCodecsSession::flush`] awaits `VideoDecoder.flush()` so the
//!    decoder emits the output for every submitted chunk. Because
//!    `decode()` is asynchronous, skipping this leaves [`drain`] with
//!    an empty or partial queue.
//! 6. [`WebCodecsSession::drain`] awaits each frame's `copyTo` promise
//!    and yields owned [`DecodedFrame`]s.
//!
//! [`drain`]: WebCodecsSession::drain
//!
//! The copy is awaited because `VideoFrame.copyTo()` resolves
//! asynchronously: the destination buffer is only valid once the
//! returned promise settles. Copying eagerly without awaiting would
//! read an unfilled buffer — a real correctness bug, not a style nit.
//!
//! Frame bytes live only in JS-heap `VideoFrame` handles and the
//! transient [`DecodedFrame`] buffers; they never enter the event log
//! (Research 01 §6.2 keeps preview frames off the persisted timeline).

use std::cell::RefCell;
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    EncodedVideoChunk, EncodedVideoChunkInit, EncodedVideoChunkType, VideoDecoder,
    VideoDecoderConfig, VideoDecoderInit, VideoFrame,
};

use crate::decoder::WebDecoder;
use crate::error::DecodeError;
use crate::error_surface::{ErrorSlot, take_pending_error};
use crate::frame::DecodedFrame;

/// Default `WebCodecs` codec string for an H.264 preview pass.
///
/// `avc1.640028` is High profile, level 4.0 — the common baseline every
/// `WebCodecs`-shipping browser decodes (Research 01 candidates §1).
const DEFAULT_H264_CODEC: &str = "avc1.640028";

/// Default `WebCodecs` codec string for an H.265 preview pass.
///
/// `hvc1.1.6.L93.B0` is Main profile, level 3.1 — the Safari 17+ /
/// Chrome 107+ HEVC decode baseline (Research 01 §R3).
const DEFAULT_H265_CODEC: &str = "hvc1.1.6.L93.B0";

impl WebDecoder {
    /// Default `WebCodecs` codec string for this decoder.
    ///
    /// Callers that need a specific profile/level pass an explicit
    /// string to [`WebCodecsSession::configure_with_codec`] instead.
    #[must_use]
    pub const fn default_codec_string(self) -> &'static str {
        match self {
            Self::H264 => DEFAULT_H264_CODEC,
            Self::H265 => DEFAULT_H265_CODEC,
        }
    }
}

/// Shared decoded-`VideoFrame` queue. The `output` callback pushes raw
/// frame handles; [`drain`] pops them, awaits each `copyTo`, and
/// converts to [`DecodedFrame`]. `Rc<RefCell<…>>` is the standard
/// single-threaded-wasm shared ownership shape — wasm32 has no threads
/// here, so there is no data race to guard against.
///
/// [`drain`]: WebCodecsSession::drain
type FrameQueue = Rc<RefCell<Vec<VideoFrame>>>;

/// A live `WebCodecs` decode session.
///
/// Holds the `VideoDecoder` and the closures it calls back into; the
/// closures must outlive the decoder, so they are stored here rather
/// than `forget()`-leaked.
pub struct WebCodecsSession {
    decoder: VideoDecoder,
    frames: FrameQueue,
    last_error: ErrorSlot,
    // Kept alive for the decoder's lifetime; dropping these would
    // invalidate the JS function pointers the decoder still holds.
    _on_output: Closure<dyn FnMut(JsValue)>,
    _on_error: Closure<dyn FnMut(JsValue)>,
}

impl WebCodecsSession {
    /// Open a decode session, wiring the `output` and `error`
    /// callbacks.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if the browser refuses
    /// to construct the `VideoDecoder` (e.g. `WebCodecs` unavailable —
    /// callers should have routed to the MSE fallback first via
    /// [`crate::codec_for_preview`]).
    pub fn new() -> Result<Self, DecodeError> {
        let frames: FrameQueue = Rc::new(RefCell::new(Vec::new()));
        let last_error: ErrorSlot = Rc::new(RefCell::new(None));

        let frames_for_output = Rc::clone(&frames);
        let on_output = Closure::wrap(Box::new(move |frame: JsValue| {
            // Retain the frame handle; the byte copy happens (and is
            // awaited) in `drain`, because `copyTo` is asynchronous.
            frames_for_output
                .borrow_mut()
                .push(frame.unchecked_into::<VideoFrame>());
        }) as Box<dyn FnMut(JsValue)>);

        let error_for_cb = Rc::clone(&last_error);
        let on_error = Closure::wrap(Box::new(move |err: JsValue| {
            *error_for_cb.borrow_mut() = Some(js_error_message(&err));
        }) as Box<dyn FnMut(JsValue)>);

        let init = VideoDecoderInit::new(
            on_error.as_ref().unchecked_ref(),
            on_output.as_ref().unchecked_ref(),
        );
        let decoder = VideoDecoder::new(&init).map_err(|e| DecodeError::DecoderInternal {
            detail: js_error_message(&e),
        })?;

        Ok(Self {
            decoder,
            frames,
            last_error,
            _on_output: on_output,
            _on_error: on_error,
        })
    }

    /// Configure the decoder for `codec` using its default
    /// profile/level string.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if the browser rejects
    /// the configuration.
    pub fn configure(&self, codec: WebDecoder) -> Result<(), DecodeError> {
        self.configure_with_codec(codec.default_codec_string())
    }

    /// Configure the decoder with an explicit `WebCodecs` codec string
    /// (e.g. `"avc1.640028"`).
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if the browser rejects
    /// the configuration.
    pub fn configure_with_codec(&self, codec_string: &str) -> Result<(), DecodeError> {
        let config = VideoDecoderConfig::new(codec_string);
        self.decoder
            .configure(&config)
            .map_err(|e| DecodeError::DecoderInternal {
                detail: js_error_message(&e),
            })
    }

    /// Submit one encoded chunk for decode.
    ///
    /// `pts_micros` is the chunk's presentation timestamp in
    /// microseconds (the `WebCodecs` `timestamp` unit). `key_frame`
    /// marks IDR / key chunks so the decoder can start a new GOP.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if chunk construction or
    /// the `decode()` call fails.
    pub fn decode_chunk(
        &self,
        data: &[u8],
        pts_micros: i64,
        key_frame: bool,
    ) -> Result<(), DecodeError> {
        let chunk_type = if key_frame {
            EncodedVideoChunkType::Key
        } else {
            EncodedVideoChunkType::Delta
        };
        let data_array = js_sys::Uint8Array::from(data);
        // WebCodecs `timestamp` is a 64-bit microsecond value. The
        // `new_with_u8_array` constructor only takes the i32 overload,
        // so seed it with 0 and set the real value through the f64
        // setter — a preview longer than ~35 min (the i32-µs cap) is a
        // valid input, not a framing error.
        let init = EncodedVideoChunkInit::new_with_u8_array(&data_array, 0, chunk_type);
        #[allow(clippy::cast_precision_loss)]
        init.set_timestamp_f64(pts_micros as f64);
        let chunk = EncodedVideoChunk::new(&init).map_err(|e| DecodeError::DecoderInternal {
            detail: js_error_message(&e),
        })?;
        self.decoder
            .decode(&chunk)
            .map_err(|e| DecodeError::DecoderInternal {
                detail: js_error_message(&e),
            })
    }

    /// Flush the decoder, blocking until every chunk submitted so far
    /// has been emitted to the `output` callback.
    ///
    /// `VideoDecoder.decode()` is asynchronous — the browser emits each
    /// `VideoFrame` on a later task, not synchronously after
    /// [`decode_chunk`]. Callers that need the frames for the chunks
    /// they submitted must `flush().await` before [`drain`], otherwise
    /// the queue is empty or partial. This wraps the `WebCodecs`
    /// `flush()` promise, which resolves once all pending output is
    /// delivered.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if the browser rejects
    /// the flush (e.g. the decoder hit a fatal error).
    ///
    /// [`decode_chunk`]: WebCodecsSession::decode_chunk
    /// [`drain`]: WebCodecsSession::drain
    pub async fn flush(&self) -> Result<(), DecodeError> {
        JsFuture::from(self.decoder.flush())
            .await
            .map(|_| ())
            .map_err(|e| DecodeError::DecoderInternal {
                detail: js_error_message(&e),
            })
    }

    /// Await every queued frame's `copyTo`, returning owned
    /// [`DecodedFrame`]s and emptying the queue.
    ///
    /// This drains only frames the decoder has *already* emitted. To
    /// retrieve the frames for chunks just submitted, call
    /// [`flush`](WebCodecsSession::flush) first — `decode()` is
    /// asynchronous, so without a flush this returns an empty or
    /// partial queue.
    ///
    /// Each `VideoFrame` is closed after its planes are copied so the
    /// JS-heap surface is released promptly.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::DecoderInternal`] if the `error` callback
    /// recorded a browser decode error, or if a frame's `allocation
    /// size` / `copyTo` fails.
    pub async fn drain(&self) -> Result<Vec<DecodedFrame>, DecodeError> {
        // Surface a recorded browser error before touching the queue, so
        // the queued frames stay intact for `Drop` to release rather
        // than being half-consumed under a fatal decode. The decision is
        // a pure, natively-tested helper (`error_surface`); only the
        // `web_sys` frame copy below cannot be faulted without a browser.
        if let Some(err) = take_pending_error(&self.last_error) {
            return Err(err);
        }
        let queued: Vec<VideoFrame> = std::mem::take(&mut self.frames.borrow_mut());
        let mut decoded = Vec::with_capacity(queued.len());
        for frame in queued {
            let result = copy_video_frame(&frame).await;
            frame.close();
            decoded.push(result?);
        }
        Ok(decoded)
    }
}

impl Drop for WebCodecsSession {
    fn drop(&mut self) {
        // `close()` releases the underlying decoder; ignore the result
        // because a session dropped after a fatal error is already
        // closed and re-closing is a benign no-op per the WebCodecs
        // spec.
        let _ = self.decoder.close();
        // Drop any frames the caller never drained so their JS-heap
        // surfaces are released.
        for frame in self.frames.borrow_mut().drain(..) {
            frame.close();
        }
    }
}

/// Copy a `WebCodecs` `VideoFrame` into an owned [`DecodedFrame`],
/// awaiting the asynchronous `copyTo`.
async fn copy_video_frame(frame: &VideoFrame) -> Result<DecodedFrame, DecodeError> {
    let width = frame.coded_width();
    let height = frame.coded_height();
    let size = frame
        .allocation_size()
        .map_err(|e| DecodeError::DecoderInternal {
            detail: js_error_message(&e),
        })? as usize;
    let mut planes = vec![0u8; size];
    let promise = frame.copy_to_with_u8_slice(&mut planes);
    JsFuture::from(promise)
        .await
        .map_err(|e| DecodeError::DecoderInternal {
            detail: js_error_message(&e),
        })?;
    Ok(DecodedFrame::new(
        width,
        height,
        planes,
        crate::frame::timestamp_to_micros(frame.timestamp()),
    ))
}

/// Extract a human-readable message from a JS error value.
fn js_error_message(value: &JsValue) -> String {
    value
        .dyn_ref::<js_sys::Error>()
        .map(|e| String::from(e.message()))
        .or_else(|| value.as_string())
        .unwrap_or_else(|| format!("{value:?}"))
}
