// composite.wgsl — YUV420p -> RGB -> z-ordered source-over composite -> YUV420p.
//
// One compute invocation per output luma pixel. The kernel walks the layer
// stack bottom-to-top (layer 0 is the bottommost), converts each layer's
// packed yuv420p sample to RGB via the pinned BT.601 limited-range matrix,
// composites it source-over the running accumulator using the layer's uniform
// alpha, then writes the final pixel back as yuv420p.
//
// Determinism contract (RenderPreset::Deterministic): the color-transform
// matrices, the layer-walk order, and the round-to-nearest quantization below
// are all pinned here. lavapipe (the software fallback adapter) evaluates the
// f32 math identically run-to-run on the same host, so two renders of the same
// inputs produce byte-identical yuv420p output — which the encoder then turns
// into a byte-identical MP4.
//
// Plane layout matches verbreel_codec_native::Frame: a packed yuv420p buffer
// of `w*h` Y bytes, then `(w/2)*(h/2)` U bytes, then the equally sized V plane,
// no inter-row padding. Bytes are addressed inside u32 words (4 bytes/word,
// little-endian within the word) because WGSL storage buffers are word-addressed.

struct Params {
    // Output luma dimensions. Both even (yuv420p chroma-subsampling constraint).
    width: u32,
    height: u32,
    // Number of layers stacked in `layers` / `layer_meta`.
    layer_count: u32,
    // Byte length of one layer's packed yuv420p buffer (w*h + 2*(w/2)*(h/2)).
    // Layers are packed back-to-back, so layer `l` starts at byte
    // `l * layer_stride_bytes` in the `layers` buffer.
    layer_stride_bytes: u32,
}

struct LayerMeta {
    // Source-over alpha for this layer, fixed-point 0..=65535 mapping 0.0..=1.0.
    // Integer carrier keeps the host->shader value exact (no f32 host rounding).
    alpha_q16: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> layers: array<u32>;
@group(0) @binding(2) var<storage, read> layer_meta: array<LayerMeta>;
@group(0) @binding(3) var<storage, read_write> out_buf: array<u32>;

// Read one byte out of the word-addressed `layers` storage buffer.
//
// WGSL forbids passing a `ptr<storage, ...>` as a function argument (without
// the unrestricted-pointer-parameters extension), so the byte readers address
// the global `layers` binding directly rather than taking a buffer pointer.
fn load_layer_byte(byte_index: u32) -> u32 {
    let word = byte_index >> 2u;
    let shift = (byte_index & 3u) * 8u;
    return (layers[word] >> shift) & 0xffu;
}

// Write one output byte. `out_buf` holds ONE u32 per output byte (not packed
// 4-per-word) specifically to avoid a read-modify-write race: if four adjacent
// luma pixels shared a word, their non-atomic RMW updates would clobber each
// other and break determinism. One word per byte makes every store a single
// independent write with no shared word, at the cost of 4x output-buffer size
// (the host repacks to bytes after readback).
fn store_byte(byte_index: u32, value: u32) {
    out_buf[byte_index] = value & 0xffu;
}

// BT.601 limited-range YCbCr (8-bit) -> RGB in 0..=1. Pinned matrix.
fn yuv_to_rgb(y: f32, u: f32, v: f32) -> vec3<f32> {
    let yy = (y - 16.0) * (255.0 / 219.0);
    let uu = u - 128.0;
    let vv = v - 128.0;
    let r = yy + 1.402 * vv * (255.0 / 224.0);
    let g = yy - 0.344136 * uu * (255.0 / 224.0) - 0.714136 * vv * (255.0 / 224.0);
    let b = yy + 1.772 * uu * (255.0 / 224.0);
    return clamp(vec3<f32>(r, g, b) / 255.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

// RGB in 0..=1 -> BT.601 limited-range YCbCr (8-bit float, pre-quantization).
fn rgb_to_yuv(rgb: vec3<f32>) -> vec3<f32> {
    let r = rgb.r * 255.0;
    let g = rgb.g * 255.0;
    let b = rgb.b * 255.0;
    let y = 16.0 + (0.299 * r + 0.587 * g + 0.114 * b) * (219.0 / 255.0);
    let u = 128.0 + (-0.168736 * r - 0.331264 * g + 0.5 * b) * (224.0 / 255.0);
    let v = 128.0 + (0.5 * r - 0.418688 * g - 0.081312 * b) * (224.0 / 255.0);
    return vec3<f32>(y, u, v);
}

// Round-to-nearest, ties away from zero, clamped to a byte. Pinned so the
// quantization is identical run-to-run.
fn quantize(value: f32) -> u32 {
    return u32(clamp(floor(value + 0.5), 0.0, 255.0));
}

// Sample layer `l` at luma pixel (px, py) and return its RGB color.
fn sample_layer_rgb(l: u32, px: u32, py: u32) -> vec3<f32> {
    let base = l * params.layer_stride_bytes;
    let w = params.width;
    let h = params.height;
    let cw = w / 2u;

    let y_byte = base + py * w + px;
    let y = f32(load_layer_byte(y_byte));

    let chroma_off = w * h;
    let cx = px / 2u;
    let cy = py / 2u;
    let u_byte = base + chroma_off + cy * cw + cx;
    let v_byte = base + chroma_off + cw * (h / 2u) + cy * cw + cx;
    let u = f32(load_layer_byte(u_byte));
    let v = f32(load_layer_byte(v_byte));

    return yuv_to_rgb(y, u, v);
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let px = gid.x;
    let py = gid.y;
    if (px >= params.width || py >= params.height) {
        return;
    }

    // Source-over composite over an opaque black background, bottom-to-top.
    var acc = vec3<f32>(0.0, 0.0, 0.0);
    for (var l: u32 = 0u; l < params.layer_count; l = l + 1u) {
        let alpha = f32(layer_meta[l].alpha_q16) / 65535.0;
        let src = sample_layer_rgb(l, px, py);
        acc = src * alpha + acc * (1.0 - alpha);
    }

    let yuv = rgb_to_yuv(acc);
    let w = params.width;
    let h = params.height;

    // Luma: one byte per invocation.
    store_byte(py * w + px, quantize(yuv.x));

    // Chroma: the top-left pixel of each 2x2 block writes the shared U/V byte.
    if ((px & 1u) == 0u && (py & 1u) == 0u) {
        let cw = w / 2u;
        let chroma_off = w * h;
        let cx = px / 2u;
        let cy = py / 2u;
        store_byte(chroma_off + cy * cw + cx, quantize(yuv.y));
        store_byte(chroma_off + cw * (h / 2u) + cy * cw + cx, quantize(yuv.z));
    }
}
