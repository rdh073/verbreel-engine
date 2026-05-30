//! Preview-session handshake tests (pure, native).
//!
//! These pin policy #405 option A end to end: a capability report
//! resolves to the exact wire literal (`"webcodecs"` / `"mse"`) and the
//! transport metadata the chosen path carries. The decision logic is a
//! `const fn`, so these run on the host with no wasm toolchain.

use verbreel_codec_web::{
    BrowserFamily, MseFallbackEnvelope, PREVIEW_CODEC_MSE, PREVIEW_CODEC_WEBCODECS,
    PreviewClientCapabilities, PreviewSessionPlan, PreviewSessionTransport, WebDecoder,
    WebPreviewCodec,
};

fn caps(family: BrowserFamily, has_webcodecs_decode: bool) -> PreviewClientCapabilities {
    PreviewClientCapabilities {
        browser_family: family,
        has_webcodecs_decode,
    }
}

#[test]
fn webcodecs_present_resolves_to_webcodecs_literal() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Other, true), WebDecoder::H264);
    assert_eq!(plan.codec, WebPreviewCodec::WebCodecs);
    assert_eq!(plan.codec_literal(), PREVIEW_CODEC_WEBCODECS);
    assert_eq!(plan.codec_literal(), "webcodecs");
    assert_eq!(plan.transport, PreviewSessionTransport::WebCodecs);
}

#[test]
fn webcodecs_absent_resolves_to_mse_literal() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Other, false), WebDecoder::H264);
    assert_eq!(plan.codec, WebPreviewCodec::MseFmp4);
    assert_eq!(plan.codec_literal(), PREVIEW_CODEC_MSE);
    assert_eq!(plan.codec_literal(), "mse");
}

#[test]
fn safari_without_webcodecs_resolves_to_mse_literal() {
    // Policy #405 option A: Safari without WebCodecs decode falls back
    // to MSE — it is NOT marked unsupported and NOT degraded to NDJSON.
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Safari, false), WebDecoder::H264);
    assert_eq!(plan.codec, WebPreviewCodec::MseFmp4);
    assert_eq!(plan.codec_literal(), "mse");
}

#[test]
fn safari_with_webcodecs_keeps_webcodecs_literal() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Safari, true), WebDecoder::H264);
    assert_eq!(plan.codec, WebPreviewCodec::WebCodecs);
    assert_eq!(plan.codec_literal(), "webcodecs");
}

#[test]
fn mse_plan_carries_fragmented_mp4_envelope() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Safari, false), WebDecoder::H264);
    let PreviewSessionTransport::Mse { envelope } = plan.transport else {
        panic!("MSE plan must carry an MSE envelope");
    };
    assert!(envelope.fragmented, "MSE segments must be fragmented MP4");
    assert!(
        envelope.mime_type.starts_with("video/mp4; codecs=\"avc1"),
        "H.264 MSE MIME must name an avc1 codec: {}",
        envelope.mime_type
    );
}

#[test]
fn mse_envelope_codec_matches_decoder_choice() {
    assert!(
        MseFallbackEnvelope::for_codec(WebDecoder::H265)
            .mime_type
            .contains("hvc1"),
        "H.265 MSE MIME must name an hvc1 codec"
    );
}

#[test]
fn webcodecs_plan_serializes_with_canonical_transport_tag() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Other, true), WebDecoder::H264);
    let json = serde_json::to_value(&plan).unwrap();
    assert_eq!(json["codec"], "webcodecs");
    assert_eq!(json["transport"]["transport"], "webcodecs");
}

#[test]
fn mse_plan_serializes_envelope_metadata_only() {
    // Only session metadata serializes — there is no frame-bytes field
    // anywhere in the plan (Research 01 §6.2).
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Other, false), WebDecoder::H264);
    let json = serde_json::to_value(&plan).unwrap();
    assert_eq!(json["codec"], "mse");
    assert_eq!(json["transport"]["transport"], "mse");
    assert_eq!(json["transport"]["envelope"]["fragmented"], true);
    assert!(json["transport"]["envelope"]["mime_type"].is_string());
}

#[test]
fn plan_round_trips_through_json() {
    let plan = PreviewSessionPlan::resolve(caps(BrowserFamily::Other, false), WebDecoder::H265);
    let json = serde_json::to_string(&plan).unwrap();
    let back: PreviewSessionPlan = serde_json::from_str(&json).unwrap();
    assert_eq!(plan, back);
}
