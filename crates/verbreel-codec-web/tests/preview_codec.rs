use verbreel_codec_web::{
    BrowserFamily, PREVIEW_CODEC_MSE, PREVIEW_CODEC_WEBCODECS, PreviewClientCapabilities,
    WebPreviewCodec, codec_for_preview, safari_fallback_codec,
};

#[test]
fn webpreviewcodec_webcodecs_constructor_uses_canonical_wire_literal() {
    let codec = WebPreviewCodec::webcodecs();
    assert_eq!(codec.wire_literal(), PREVIEW_CODEC_WEBCODECS);
    assert_eq!(serde_json::to_string(&codec).unwrap(), "\"webcodecs\"");
}

#[test]
fn webpreviewcodec_mse_constructor_uses_canonical_wire_literal() {
    let codec = WebPreviewCodec::mse_fmp4();
    assert_eq!(codec.wire_literal(), PREVIEW_CODEC_MSE);
    assert_eq!(serde_json::to_string(&codec).unwrap(), "\"mse\"");
}

#[test]
fn safari_without_webcodecs_falls_back_to_mse() {
    let caps = PreviewClientCapabilities {
        browser_family: BrowserFamily::Safari,
        has_webcodecs_decode: false,
    };
    assert_eq!(codec_for_preview(caps), WebPreviewCodec::mse_fmp4());
}

#[test]
fn safari_with_webcodecs_keeps_webcodecs_primary() {
    let caps = PreviewClientCapabilities {
        browser_family: BrowserFamily::Safari,
        has_webcodecs_decode: true,
    };
    assert_eq!(codec_for_preview(caps), WebPreviewCodec::webcodecs());
}

#[test]
fn non_safari_without_webcodecs_uses_same_mse_fallback() {
    let caps = PreviewClientCapabilities {
        browser_family: BrowserFamily::Other,
        has_webcodecs_decode: false,
    };
    assert_eq!(codec_for_preview(caps), WebPreviewCodec::mse_fmp4());
}

#[test]
fn safari_fallback_api_is_pinned_to_mse_fmp4() {
    assert_eq!(
        safari_fallback_codec(BrowserFamily::Safari),
        WebPreviewCodec::mse_fmp4()
    );
    assert_eq!(
        safari_fallback_codec(BrowserFamily::Other),
        WebPreviewCodec::mse_fmp4()
    );
}
