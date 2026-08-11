#import bevy_pbr::forward_io::VertexOutput

struct CharmeMaterialParams {
    lanes: array<vec4<f32>, 16>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: CharmeMaterialParams;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let controls = material.lanes[0];
    let tint = material.lanes[1];
    let bands = max(bitcast<u32>(controls.w), 1u);
    let quantized = floor((0.25 + 0.75 * abs(in.world_normal.y)) * f32(bands)) / f32(bands);
    let response = quantized * (0.55 + 0.45 * (1.0 - controls.x));
    let rim = controls.y * pow(1.0 - abs(in.world_normal.z), 2.0);
    let outline = 1.0 + min(controls.z, 10.0) * 0.015;
    return vec4<f32>(tint.rgb * (response * outline + rim), tint.a);
}
