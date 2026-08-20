// Fullscreen pass compositing the selection wire mask over the main frame.
//
// The renderer's whole pipeline works in raw byte space (the view target
// textures are written and sampled without sRGB conversion), so the composite
// blends the mask over the main texture without any color-space conversion.
// Scene pixels pass through unchanged.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var main_texture: texture_2d<f32>;
@group(0) @binding(1) var wire_mask: texture_2d<f32>;
@group(0) @binding(2) var main_sampler: sampler;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let main_color = textureSample(main_texture, main_sampler, in.uv);
    let mask = textureSample(wire_mask, main_sampler, in.uv);
    let rgb = main_color.rgb * (1.0 - mask.a) + mask.rgb * mask.a;
    return vec4(rgb, 1.0);
}
