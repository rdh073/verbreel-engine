// Spike S1 slice C — alpha-blend two RGB textures by a uniform weight.
//
//   out = mix(a, b, weight) = a * (1 - weight) + b * weight
//
// weight ∈ [0, 1]. Passed as a uniform.
//
// For in-the-clear regions (weight = 0 → pure A; weight = 1 → pure B), the
// orchestrator does NOT dispatch this shader — it picks the appropriate
// `inv_bg` directly off `rgb_a`. This shader is only dispatched inside the
// crossfade region (weight strictly in (0, 1)).
//
// The padding fields satisfy WebGPU's 16-byte uniform-buffer struct alignment.

struct BlendUniform {
    weight: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0) var src_a: texture_2d<f32>;
@group(0) @binding(1) var src_b: texture_2d<f32>;
@group(0) @binding(2) var rgb_out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(3) var<uniform> u: BlendUniform;

@compute @workgroup_size(8, 8, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(rgb_out);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }

    let coord = vec2<i32>(i32(gid.x), i32(gid.y));
    let a = textureLoad(src_a, coord, 0).rgb;
    let b = textureLoad(src_b, coord, 0).rgb;
    let mix_rgb = mix(a, b, u.weight);
    textureStore(rgb_out, coord, vec4<f32>(mix_rgb, 1.0));
}
