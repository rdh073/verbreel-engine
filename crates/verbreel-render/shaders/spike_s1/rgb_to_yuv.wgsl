// Spike S1 slice B — RGBA8 → YUV420P (BT.709 limited range)
//
// Inverse of yuv_to_rgb.wgsl. RGB inputs in [0,1] full-range; output:
//   Y'  = 0.18259 * R + 0.61423 * G + 0.06201 * B + 16/255
//   U'  = -0.10064 * R - 0.33856 * G + 0.43922 * B + 128/255
//   V'  =  0.43922 * R - 0.39894 * G - 0.04027 * B + 128/255
//
// Each invocation writes ONE Y sample plus, for every 2×2 luma quad,
// ONE U and V sample (computed by box-average of the four covered RGB
// pixels). To keep work uniform across threads, we run a 2D dispatch
// sized to the LUMA dimensions and have the (gid.x % 2 == 0 && gid.y
// % 2 == 0) thread write the chroma sample for its quad.

@group(0) @binding(0) var rgb_in: texture_2d<f32>;
@group(0) @binding(1) var y_out:  texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(2) var u_out:  texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var v_out:  texture_storage_2d<rgba8unorm, write>;

fn rgb_to_y(c: vec3<f32>) -> f32 {
    return 0.18259 * c.r + 0.61423 * c.g + 0.06201 * c.b + 16.0 / 255.0;
}
fn rgb_to_u(c: vec3<f32>) -> f32 {
    return -0.10064 * c.r - 0.33856 * c.g + 0.43922 * c.b + 128.0 / 255.0;
}
fn rgb_to_v(c: vec3<f32>) -> f32 {
    return  0.43922 * c.r - 0.39894 * c.g - 0.04027 * c.b + 128.0 / 255.0;
}

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims_y = textureDimensions(y_out);
    if (gid.x >= dims_y.x || gid.y >= dims_y.y) { return; }

    let c = textureLoad(rgb_in, vec2<i32>(i32(gid.x), i32(gid.y)), 0).rgb;
    let y_val = clamp(rgb_to_y(c), 0.0, 1.0);
    textureStore(y_out, vec2<i32>(i32(gid.x), i32(gid.y)), vec4<f32>(y_val, 0.0, 0.0, 1.0));

    // Chroma: written by the top-left thread of each 2×2 luma quad
    if (gid.x % 2u == 0u && gid.y % 2u == 0u) {
        let c00 = textureLoad(rgb_in, vec2<i32>(i32(gid.x),     i32(gid.y)),     0).rgb;
        let c10 = textureLoad(rgb_in, vec2<i32>(i32(gid.x + 1u), i32(gid.y)),     0).rgb;
        let c01 = textureLoad(rgb_in, vec2<i32>(i32(gid.x),     i32(gid.y + 1u)), 0).rgb;
        let c11 = textureLoad(rgb_in, vec2<i32>(i32(gid.x + 1u), i32(gid.y + 1u)), 0).rgb;
        let avg = (c00 + c10 + c01 + c11) * 0.25;

        let u_val = clamp(rgb_to_u(avg), 0.0, 1.0);
        let v_val = clamp(rgb_to_v(avg), 0.0, 1.0);

        let chroma_xy = vec2<i32>(i32(gid.x / 2u), i32(gid.y / 2u));
        textureStore(u_out, chroma_xy, vec4<f32>(u_val, 0.0, 0.0, 1.0));
        textureStore(v_out, chroma_xy, vec4<f32>(v_val, 0.0, 0.0, 1.0));
    }
}
