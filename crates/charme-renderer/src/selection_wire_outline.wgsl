// Screen-space silhouette outline for the selection mask camera.
//
// Mirrors Blender's overlay outline pass (overlay_outline_*.glsl): Blender
// detects the outline as object-id boundaries in a dedicated id buffer. The
// mask camera renders only the selected object, so its depth buffer is the id
// buffer: pixels with depth > 0 belong to the object, cleared pixels are the
// background. Boundary pixels between the two form exactly the object's outer
// silhouette (self-occluded parts share the object "id" and stay outline-free,
// matching Blender). Because other objects never enter the mask, the outline
// inherits the wireframe's x-ray behavior. Depth is only used for the
// object/background classification, never as a discontinuity metric.
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var depth_tx: texture_depth_2d;

const OUTLINE_COLOR: vec4<f32> = vec4(1.0, 0.42, 0.02, 1.0);
// Reverse-Z clears the mask depth to 0.0 (infinite distance).
const BACKGROUND_DEPTH: f32 = 1e-6;

fn is_background(pixel: vec2<i32>) -> bool {
    return textureLoad(depth_tx, pixel, 0) <= BACKGROUND_DEPTH;
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(depth_tx));
    let pixel = vec2<i32>(in.position.xy);
    let center_bg = is_background(pixel);

    let offsets = array<vec2<i32>, 4>(
        vec2(1, 0),
        vec2(-1, 0),
        vec2(0, 1),
        vec2(0, -1),
    );
    var silhouette = false;
    for (var i = 0; i < 4; i++) {
        let q = pixel + offsets[i];
        if (any(q < vec2<i32>(0, 0)) || any(q >= size)) {
            continue;
        }
        if (is_background(q) != center_bg) {
            silhouette = true;
            break;
        }
    }
    if (!silhouette) {
        return vec4(0.0);
    }
    return OUTLINE_COLOR;
}
