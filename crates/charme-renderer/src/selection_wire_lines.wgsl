// Wireframe line rendering for the selection mask camera.
//
// Each line segment is baked on the CPU as 6 vertices (two triangles) carrying
// both endpoints plus the quad corner id. The vertex shader expands the
// segment into a screen-space quad of `LINE_WIDTH_PX` pixels, following the
// instanced-lines algorithm from https://wwwtyro.net/2019/11/18/instanced-lines.html
// (the same approach used by bevy_gizmos).
//
// Depth handling: the mask camera's depth buffer contains only the selected
// object (written by the depth prepass of the mask schedule), so the hardware
// depth test of the line pipeline provides "occluded by itself, never by
// other objects" for free. `LINE_DEPTH_EPSILON` pushes the line by a tiny
// constant NDC offset towards the camera so that fragments lying exactly on
// the object surface pass the depth comparison, while genuinely closer
// surfaces keep occluding the line.
#import bevy_render::view::View

// View uniform from the pipeline's view bind group.
@group(0) @binding(0) var<uniform> view: View;

const LINE_WIDTH_PX: f32 = 3.5;
const LINE_COLOR: vec4<f32> = vec4(1.0, 0.42, 0.02, 1.0);
const NEAR_PLANE_EPSILON: f32 = 4.88e-04;
// Constant NDC offset towards the camera: just enough for line fragments to
// win against the surface they lie on (equal depth), yet small enough that
// genuinely closer surfaces still occlude the line.
const LINE_DEPTH_EPSILON: f32 = 1e-4;
// Half of the feather distance in pixels: the outermost pixels of the quad
// fade out to zero alpha, producing smooth line edges (the mask camera runs
// without MSAA so the outline pass can read its depth with `textureLoad`).
const EDGE_FEATHER_PX: f32 = 0.75;

struct VertexInput {
    @location(0) position_a: vec3<f32>,
    @location(1) position_b: vec3<f32>,
    @location(2) corner: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) line_fraction: f32,
};

@vertex
fn vertex(vertex: VertexInput) -> VertexOutput {
    var positions = array<vec2<f32>, 6>(
        vec2(-0.5, 0.0),
        vec2(-0.5, 1.0),
        vec2(0.5, 1.0),
        vec2(-0.5, 0.0),
        vec2(0.5, 1.0),
        vec2(0.5, 0.0),
    );
    let position = positions[vertex.corner];

    // Selection geometry is stored in world space with the same translation
    // applied to the PMX scene, so no model transform is needed here.
    var clip_a = view.clip_from_world * vec4(vertex.position_a, 1.0);
    var clip_b = view.clip_from_world * vec4(vertex.position_b, 1.0);

    // Manual near plane clipping to avoid errors when doing the perspective
    // divide inside this shader.
    clip_a = clip_near_plane(clip_a, clip_b);
    clip_b = clip_near_plane(clip_b, clip_a);
    let clip = mix(clip_a, clip_b, position.y);

    let resolution = view.viewport.zw;
    let screen_a = resolution * (0.5 * clip_a.xy / clip_a.w + 0.5);
    let screen_b = resolution * (0.5 * clip_b.xy / clip_b.w + 0.5);

    let y_basis = normalize(screen_b - screen_a);
    let x_basis = vec2(-y_basis.y, y_basis.x);

    let x_offset = LINE_WIDTH_PX * position.x * x_basis;
    let screen = mix(screen_a, screen_b, position.y) + x_offset;

    let depth = clip.z + LINE_DEPTH_EPSILON * clip.w;

    let clip_position = vec4(clip.w * ((2.0 * screen) / resolution - 1.0), depth, clip.w);

    return VertexOutput(clip_position, position.x * 2.0);
}

fn clip_near_plane(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    // Move a if a is behind the near plane and b is in front.
    if a.z > a.w && b.z <= b.w {
        let distance_a = a.z - a.w;
        let distance_b = b.z - b.w;
        let t = distance_a / (distance_a - distance_b) + NEAR_PLANE_EPSILON;
        return mix(a, b, t);
    }
    return a;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let distance_px = abs(in.line_fraction) * LINE_WIDTH_PX * 0.5;
    let coverage = clamp(
        (LINE_WIDTH_PX * 0.5 - distance_px) / EDGE_FEATHER_PX,
        0.0,
        1.0,
    );
    return vec4(LINE_COLOR.rgb, coverage);
}
