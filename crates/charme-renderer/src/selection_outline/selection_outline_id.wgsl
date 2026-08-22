// Object-ID material fragment shader.
//
// The object ID is baked into each primitive's vertex color (ATTRIBUTE_COLOR)
// as a constant per-vertex value; `u32(input.color.r)` recovers it exactly
// for IDs below 2^24. The value is already packed like Blender's
// `overlay_outline_prepass` (top 2 bits = color class, low 14 bits = object
// ID) and is written verbatim to the red/green bytes of an `Rgba8Unorm`
// target. The detect pass decodes those bytes without rounding loss.

#import bevy_pbr::forward_io::VertexOutput

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
#ifdef VERTEX_COLORS
    let packed = u32(input.color.r);
    let r = f32(packed & 0xFFu) / 255.0;
    let g = f32((packed >> 8u) & 0xFFu) / 255.0;
    return vec4(r, g, 0.0, 1.0);
#else
    return vec4(0.0, 0.0, 0.0, 1.0);
#endif
}