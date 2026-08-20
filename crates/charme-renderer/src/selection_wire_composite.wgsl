// Fullscreen pass compositing the selection wire mask over the main frame.
//
// The scene is written to the main view target in sRGB storage; sampling it
// decodes to linear. The mask stores the wireframe color in linear storage
// (Rgba8Unorm) with straight alpha coverage, so no decode happens there. The
// blend runs in linear space and the result is re-encoded to the storage
// space, so scene pixels pass through unchanged.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var main_texture: texture_2d<f32>;
@group(0) @binding(1) var wire_mask: texture_2d<f32>;
@group(0) @binding(2) var main_sampler: sampler;

fn encode_srgb(color: vec3<f32>) -> vec3<f32> {
    let linear = max(color, vec3(0.0));
    return select(
        12.92 * linear,
        1.055 * pow(linear, vec3(1.0 / 2.4)) - 0.055,
        linear <= vec3(0.0031308),
    );
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let main_color = textureSample(main_texture, main_sampler, in.uv);
    let mask = textureSample(wire_mask, main_sampler, in.uv);
    let linear = main_color.rgb * (1.0 - mask.a) + mask.rgb * mask.a;
    return vec4(encode_srgb(linear), 1.0);
}
