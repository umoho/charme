// Fullscreen object-ID edge detection and compositing.
//
// Mirrors Blender's overlay outline resolve pass
// (overlay_outline_detect_frag.glsl): the center pixel's object ID is compared
// against its four neighbours. A differing neighbour marks an object
// boundary, which is painted with the orange outline. Because only selected
// primitives carry a non-zero ID, the outline inherits the x-ray behavior:
// other objects never occlude it. ID 0 is the background.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var main_texture: texture_2d<f32>;
@group(0) @binding(1) var id_texture: texture_2d<f32>;
@group(0) @binding(2) var main_sampler: sampler;

const OUTLINE_COLOR: vec4<f32> = vec4(1.0, 0.42, 0.02, 1.0);

fn decode_id(pixel: vec2<i32>) -> u32 {
    let color = textureLoad(id_texture, pixel, 0);
    let r = u32(color.r * 255.0 + 0.5);
    let g = u32(color.g * 255.0 + 0.5);
    return r | (g << 8u);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let main_color = textureSample(main_texture, main_sampler, in.uv);
    let pixel = vec2<i32>(in.position.xy);
    let size = vec2<i32>(textureDimensions(id_texture));

    let center = decode_id(pixel);
    if center == 0u {
        // Background pixels never draw the outline.
        return vec4(main_color.rgb, 1.0);
    }

    let offsets = array<vec2<i32>, 4>(
        vec2(1, 0),
        vec2(-1, 0),
        vec2(0, 1),
        vec2(0, -1),
    );
    var boundary = false;
    for (var i = 0; i < 4; i++) {
        let q = pixel + offsets[i];
        if (any(q < vec2<i32>(0, 0)) || any(q >= size)) {
            // Reaching the viewport edge is still an object boundary.
            boundary = true;
            break;
        }
        if decode_id(q) != center {
            boundary = true;
            break;
        }
    }

    if boundary {
        let rgb = main_color.rgb * (1.0 - OUTLINE_COLOR.a) + OUTLINE_COLOR.rgb * OUTLINE_COLOR.a;
        return vec4(rgb, 1.0);
    }
    return vec4(main_color.rgb, 1.0);
}