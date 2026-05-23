// Spike S1 slice B — YUV420P → RGBA8 (BT.709 limited range)
//
// Bindings:
//   group(0) binding(0): texture_2d<f32>  y_plane    (R8Unorm, sample)
//   group(0) binding(1): texture_2d<f32>  u_plane    (R8Unorm, sample, half-res)
//   group(0) binding(2): texture_2d<f32>  v_plane    (R8Unorm, sample, half-res)
//   group(0) binding(3): texture_storage_2d<rgba8unorm, write> rgb_out
//
// Limited range: Y∈[16/255, 235/255], UV∈[16/255, 240/255]; expand to [0,1].
// BT.709 matrix (limited-range to full-range RGB):
//   R = 1.16438 * (Y' - 16/255)                            + 1.79274 * (V' - 128/255)
//   G = 1.16438 * (Y' - 16/255) - 0.21325 * (U' - 128/255) - 0.53291 * (V' - 128/255)
//   B = 1.16438 * (Y' - 16/255) + 2.11240 * (U' - 128/255)
//
// Workgroup size 8×8 covers a 16×16 luma block, which maps to one 8×8 chroma block.
// No filtering: chroma is nearest-sampled (each 2×2 luma quad shares one UV value).

@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var u_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;
@group(0) @binding(3) var rgb_out: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims_rgb = textureDimensions(rgb_out);
    if (gid.x >= dims_rgb.x || gid.y >= dims_rgb.y) { return; }

    let y_sample = textureLoad(y_plane, vec2<i32>(i32(gid.x), i32(gid.y)), 0).r;
    let chroma_xy = vec2<i32>(i32(gid.x / 2u), i32(gid.y / 2u));
    let u_sample = textureLoad(u_plane, chroma_xy, 0).r;
    let v_sample = textureLoad(v_plane, chroma_xy, 0).r;

    let y = 1.16438 * (y_sample - 16.0/255.0);
    let u = u_sample - 128.0/255.0;
    let v = v_sample - 128.0/255.0;

    let r = clamp(y + 1.79274 * v, 0.0, 1.0);
    let g = clamp(y - 0.21325 * u - 0.53291 * v, 0.0, 1.0);
    let b = clamp(y + 2.11240 * u, 0.0, 1.0);

    textureStore(rgb_out, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(r, g, b, 1.0));
}
