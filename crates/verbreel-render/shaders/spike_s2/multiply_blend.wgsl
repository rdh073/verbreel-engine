// Spike S2 — multiply blend of two RGBA textures.
//
// out.rgb = a.rgb * b.rgb        (per-channel multiply, linear space)
// out.a   = 1.0
//
// No branches, no loops, no built-in derivatives. Pure arithmetic.
// This is the spec §11 S2 reference shader; identical bytes on both
// native (Vulkan via Naga) and Chrome WebGPU.

@group(0) @binding(0) var src_a: texture_2d<f32>;
@group(0) @binding(1) var src_b: texture_2d<f32>;
@group(0) @binding(2) var dst:   texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(dst);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(src_a, coord, 0);
    let b = textureLoad(src_b, coord, 0);
    textureStore(dst, coord, vec4<f32>(a.rgb * b.rgb, 1.0));
}
