//! Preview-session bridge from [`crate::EngineHandle`] to
//! `verbreel-codec-web`.
//!
//! Decision #404 keeps the browser embedding preview-only: this module
//! is the adapter that turns a capability report into a live codec-web
//! decode session and exposes `open` / `seek` / `frame_at` to JS.
//! Mutation and persistence stay on native/HTTP/MCP — any attempt to
//! route an edit through the browser surfaces
//! [`WasmError::BrowserNoPersistence`].
//!
//! ## Why a separate type from `EngineHandle`
//!
//! [`EngineHandle`](crate::EngineHandle) is `Send + Sync` (its
//! type-surface test pins that) so JS bridge code can hold it across an
//! async boundary. The codec-web [`WebCodecsSession`] is `!Send`: it
//! owns `Rc<RefCell<…>>` queues and `wasm-bindgen` `Closure`s the
//! browser calls back into. Storing the session inside `EngineHandle`
//! would make the handle `!Send` and break that contract. So the handle
//! is a factory — [`EngineHandle::open_preview_session`] mints a
//! `PreviewSession` that owns the `!Send` decoder, and the handle itself
//! stays thread-safe.
//!
//! ## Native vs wasm32
//!
//! The transport decision ([`PreviewSessionPlan::resolve`]) and the
//! tick→microsecond conversion ([`tick_to_micros`]) are pure and compile
//! on every target, so they carry native unit tests. The live decode
//! loop ([`WebCodecsSession`]) exists only on wasm32; on native targets
//! `frame_at` / `seek` operate on the plan and timing state alone, which
//! is enough to test the bridge's control flow without a browser.
//!
//! Frame bytes never enter the event log: the encoded chunks arrive
//! through [`PreviewSession::frame_at`]'s `chunk` argument (streamed by
//! the JS transport, Research 01 §6.2) and decoded frames live only in
//! the transient [`PreviewFrame`] buffer.

#[cfg(target_arch = "wasm32")]
use verbreel_codec_web::WebCodecsSession;
use verbreel_codec_web::{DecodedFrame, PreviewClientCapabilities, PreviewSessionPlan, WebDecoder};
use verbreel_ir::TICK_RATE_HZ;
use wasm_bindgen::prelude::wasm_bindgen;

use crate::error::WasmError;

/// Convert an engine tick (240,000 Hz base, §0.2) to the microsecond
/// presentation timestamp `WebCodecs` uses (`VideoFrame.timestamp`).
///
/// Exact for the tick rate: `1_000_000 / 240_000 = 25 / 6`, so
/// `micros = tick * 25 / 6`. Computed in `i128` to keep the
/// intermediate `tick * 25` from overflowing `i64` for very large
/// ticks; the result is clamped back to `i64` because the codec-web
/// `decode_chunk` timestamp unit is `i64` microseconds.
///
/// Lives here (not behind `cfg(wasm32)`) so the conversion is exercised
/// by native unit tests rather than only on a browser run.
#[must_use]
pub fn tick_to_micros(tick: i64) -> i64 {
    let micros = i128::from(tick) * 1_000_000 / i128::from(TICK_RATE_HZ);
    let clamped = micros.clamp(i128::from(i64::MIN), i128::from(i64::MAX));
    // Clamped to exactly the i64 range on the line above, so the cast
    // cannot truncate — the saturation case is handled by `clamp`, not
    // by `as`.
    #[allow(clippy::cast_possible_truncation)]
    let micros_i64 = clamped as i64;
    micros_i64
}

/// One decoded preview frame exposed to JS.
///
/// Opaque handle over codec-web's [`DecodedFrame`]: it carries the
/// dimensions and presentation timestamp the JS render loop branches on,
/// plus a borrowed view of the plane bytes for upload. The plane layout
/// is codec-web's (flat `Vec<u8>`, format negotiated in Spike S2); this
/// type does not reinterpret it.
#[wasm_bindgen]
pub struct PreviewFrame {
    inner: DecodedFrame,
}

#[wasm_bindgen]
impl PreviewFrame {
    /// Pixel width of the decoded frame.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn width(&self) -> u32 {
        self.inner.width()
    }

    /// Pixel height of the decoded frame.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn height(&self) -> u32 {
        self.inner.height()
    }

    /// Presentation timestamp in microseconds (the `WebCodecs`
    /// `VideoFrame.timestamp` unit).
    #[wasm_bindgen(getter, js_name = ptsMicros)]
    #[must_use]
    pub fn pts_micros(&self) -> u64 {
        self.inner.pts_micros()
    }

    /// Copy of the decoded plane bytes for GPU/canvas upload. A copy
    /// (not a borrow) because `wasm-bindgen` getters cannot hand JS a
    /// view that aliases Rust-owned memory across the call boundary.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn planes(&self) -> Vec<u8> {
        self.inner.planes().to_vec()
    }
}

impl PreviewFrame {
    /// Wrap a codec-web decoded frame. Crate-internal: the wasm32
    /// `PreviewSession::frameAt` decode loop is the production producer;
    /// native tests construct one directly to exercise the getters.
    // Called from the wasm32-only decode loop and from the native unit
    // tests below; unused on a native non-test lib build (mirrors
    // codec-web's `timestamp_to_micros`).
    #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(dead_code))]
    #[must_use]
    pub(crate) const fn new(inner: DecodedFrame) -> Self {
        Self { inner }
    }
}

/// A browser preview decode session bridging to `verbreel-codec-web`.
///
/// Constructed via [`crate::EngineHandle::open_preview_session`]. Holds
/// the resolved transport plan and the current seek position; on wasm32
/// it also owns the live [`WebCodecsSession`]. `!Send` on wasm32 by way
/// of the decoder's `Rc`/`Closure` fields — preview decode is a
/// single-threaded browser concern, never crossed over a thread
/// boundary.
#[wasm_bindgen]
pub struct PreviewSession {
    plan: PreviewSessionPlan,
    codec: WebDecoder,
    seek_micros: i64,
    #[cfg(target_arch = "wasm32")]
    decoder: WebCodecsSession,
}

impl PreviewSession {
    /// Open a preview session for `codec` against the client's reported
    /// `caps`.
    ///
    /// Resolves the transport plan (policy #405 via codec-web) and, on
    /// wasm32, constructs and configures the live `WebCodecs` decoder.
    /// The seek position starts at tick 0.
    ///
    /// # Errors
    ///
    /// On wasm32, returns [`WasmError::PreviewDecode`] if the browser
    /// refuses to construct or configure the `VideoDecoder`. On native
    /// targets this is infallible (no decoder is built), but the
    /// signature returns `Result` on every target so JS callers see one
    /// shape.
    // The native branch never constructs the fallible decoder, so the
    // wrap looks unnecessary to clippy on that target — but the `Result`
    // is load-bearing on wasm32 and the cross-target signature must match.
    #[cfg_attr(not(target_arch = "wasm32"), allow(clippy::unnecessary_wraps))]
    pub(crate) fn open(
        caps: PreviewClientCapabilities,
        codec: WebDecoder,
    ) -> Result<Self, WasmError> {
        let plan = PreviewSessionPlan::resolve(caps, codec);

        #[cfg(target_arch = "wasm32")]
        {
            let decoder = WebCodecsSession::new().map_err(|e| WasmError::from_decode(&e))?;
            decoder
                .configure(codec)
                .map_err(|e| WasmError::from_decode(&e))?;
            Ok(Self {
                plan,
                codec,
                seek_micros: 0,
                decoder,
            })
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(Self {
                plan,
                codec,
                seek_micros: 0,
            })
        }
    }
}

#[wasm_bindgen]
impl PreviewSession {
    /// The chosen transport's canonical wire literal (`"webcodecs"` or
    /// `"mse"`).
    #[wasm_bindgen(getter, js_name = transportLiteral)]
    #[must_use]
    pub fn transport_literal(&self) -> String {
        self.plan.codec_literal().to_string()
    }

    /// The current seek position in microseconds.
    #[wasm_bindgen(getter, js_name = seekMicros)]
    #[must_use]
    pub fn seek_micros(&self) -> i64 {
        self.seek_micros
    }

    /// The decode codec selected for this session (`"h264"` or
    /// `"h265"`), matching codec-web's `WebDecoder` serde token.
    #[wasm_bindgen(getter, js_name = codecLiteral)]
    #[must_use]
    pub fn codec_literal(&self) -> String {
        match self.codec {
            WebDecoder::H264 => "h264",
            WebDecoder::H265 => "h265",
        }
        .to_string()
    }

    /// Seek the preview to engine tick `at_tk` (240,000 Hz base).
    ///
    /// Sets the presentation timestamp the next `frameAt` decode is
    /// tagged with. Seeking does not itself decode — it positions the
    /// session so the JS transport can feed the chunk covering that
    /// tick.
    #[wasm_bindgen]
    pub fn seek(&mut self, at_tk: i64) {
        self.seek_micros = tick_to_micros(at_tk);
    }
}

/// The live decode entry point. `async` `wasm-bindgen` exports expand to
/// code that references `wasm-bindgen-futures` (a wasm32-only dep), so
/// this method is gated to wasm32. Native callers exercise the bridge's
/// pure control flow (seek positioning, transport plan) directly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
impl PreviewSession {
    /// Decode the encoded `chunk` covering the current seek position and
    /// return the first frame it produces, if any.
    ///
    /// `key_frame` marks an IDR/keyframe chunk so the decoder can start a
    /// new GOP — JS sets it for the first chunk after a seek. The chunk
    /// is tagged with the seek position's microsecond timestamp.
    ///
    /// Returns `None` when the decoder emitted no frame for the submitted
    /// chunk (e.g. a delta chunk the decoder buffers, or an empty
    /// flush). The encoded bytes arrive here from the JS transport and
    /// never touch the event log (Research 01 §6.2).
    ///
    /// # Errors
    ///
    /// Returns [`WasmError::PreviewDecode`] if the browser decode,
    /// flush, or frame copy fails.
    #[wasm_bindgen(js_name = frameAt)]
    pub async fn frame_at(
        &mut self,
        chunk: &[u8],
        key_frame: bool,
    ) -> Result<Option<PreviewFrame>, WasmError> {
        self.decoder
            .decode_chunk(chunk, self.seek_micros, key_frame)
            .map_err(|e| WasmError::from_decode(&e))?;
        self.decoder
            .flush()
            .await
            .map_err(|e| WasmError::from_decode(&e))?;
        let frames = self
            .decoder
            .drain()
            .await
            .map_err(|e| WasmError::from_decode(&e))?;
        Ok(frames.into_iter().next().map(PreviewFrame::new))
    }
}

impl WasmError {
    /// Map a codec-web [`verbreel_codec_web::DecodeError`] onto the flat
    /// wasm error surface. Kept here (not a `From` impl) so the codec-web
    /// enum stays an implementation detail of the bridge rather than part
    /// of the public `WasmError` conversion surface.
    #[cfg(target_arch = "wasm32")]
    fn from_decode(err: &verbreel_codec_web::DecodeError) -> Self {
        Self::PreviewDecode {
            detail: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewFrame, PreviewSession, tick_to_micros};
    use verbreel_codec_web::{BrowserFamily, DecodedFrame, PreviewClientCapabilities, WebDecoder};

    fn caps(has_webcodecs: bool, safari: bool) -> PreviewClientCapabilities {
        PreviewClientCapabilities {
            browser_family: if safari {
                BrowserFamily::Safari
            } else {
                BrowserFamily::Other
            },
            has_webcodecs_decode: has_webcodecs,
        }
    }

    // --- tick_to_micros (pure, §0.2 240 kHz → WebCodecs µs) --------------

    #[test]
    fn tick_zero_is_zero_micros() {
        assert_eq!(tick_to_micros(0), 0);
    }

    #[test]
    fn one_second_of_ticks_is_one_million_micros() {
        // 240_000 ticks = 1 s = 1_000_000 µs.
        assert_eq!(tick_to_micros(240_000), 1_000_000);
    }

    #[test]
    fn tick_conversion_is_exact_for_the_rate() {
        // 25/6 is exact: 6 ticks = 25 µs.
        assert_eq!(tick_to_micros(6), 25);
        assert_eq!(tick_to_micros(48_000), 200_000); // 1/5 s
    }

    #[test]
    fn negative_ticks_map_to_negative_micros() {
        assert_eq!(tick_to_micros(-240_000), -1_000_000);
    }

    #[test]
    fn large_tick_does_not_overflow_i64_intermediate() {
        // tick * 1_000_000 overflows i64 before the divide; the i128
        // intermediate must keep it intact. i64::MAX ticks → a value
        // that still fits i64 after the /240_000.
        let got = tick_to_micros(i64::MAX);
        assert!(got > 0, "huge positive tick must stay positive, got {got}");
    }

    // --- PreviewSession::open / transport plan --------------------------

    #[test]
    fn open_with_webcodecs_selects_webcodecs_transport() {
        let session = PreviewSession::open(caps(true, false), WebDecoder::H264)
            .expect("native open is infallible");
        assert_eq!(session.transport_literal(), "webcodecs");
        assert_eq!(session.codec_literal(), "h264");
        assert_eq!(session.seek_micros(), 0);
    }

    #[test]
    fn open_without_webcodecs_falls_back_to_mse() {
        let session = PreviewSession::open(caps(false, false), WebDecoder::H265)
            .expect("native open is infallible");
        assert_eq!(session.transport_literal(), "mse");
        assert_eq!(session.codec_literal(), "h265");
    }

    #[test]
    fn safari_without_webcodecs_falls_back_to_mse_not_unsupported() {
        // Pins #376 decision A through the bridge: Safari without
        // WebCodecs degrades to MSE, never "unsupported".
        let session = PreviewSession::open(caps(false, true), WebDecoder::H264)
            .expect("native open is infallible");
        assert_eq!(session.transport_literal(), "mse");
    }

    // --- seek positioning -----------------------------------------------

    #[test]
    fn seek_sets_micros_from_tick() {
        let mut session = PreviewSession::open(caps(true, false), WebDecoder::H264)
            .expect("native open is infallible");
        session.seek(240_000);
        assert_eq!(session.seek_micros(), 1_000_000);
        session.seek(0);
        assert_eq!(session.seek_micros(), 0);
    }

    // --- PreviewFrame getters -------------------------------------------

    #[test]
    fn preview_frame_exposes_decoded_frame_fields() {
        let frame = PreviewFrame::new(DecodedFrame::new(1920, 1080, vec![1, 2, 3, 4], 42));
        assert_eq!(frame.width(), 1920);
        assert_eq!(frame.height(), 1080);
        assert_eq!(frame.pts_micros(), 42);
        assert_eq!(frame.planes(), vec![1, 2, 3, 4]);
    }
}
