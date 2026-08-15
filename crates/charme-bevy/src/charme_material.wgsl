#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_view_bindings as view_bindings,
    mesh_view_types,
    shadows,
}

struct CharmeMaterialParams {
    lanes: array<vec4<f32>, 16>,
};

@group(#{MATERIAL_BIND_GROUP}) @binding(0)
var<uniform> material: CharmeMaterialParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1)
var base_color_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2)
var base_color_sampler: sampler;

fn safe_normalize(value: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let magnitude = length(value);
    if magnitude > 0.0001 {
        return value / magnitude;
    }
    return fallback;
}

fn main_light_direction() -> vec3<f32> {
    if view_bindings::lights.n_directional_lights > 0u {
        return safe_normalize(
            view_bindings::lights.directional_lights[0u].direction_to_light,
            vec3<f32>(0.0, 1.0, 0.0),
        );
    }

    // Material-ball previews may not have a directional light. Keep a stable
    // studio direction so they still receive readable shading.
    return safe_normalize(vec3<f32>(-0.35, 0.8, 0.45), vec3<f32>(0.0, 1.0, 0.0));
}

fn main_light_color() -> vec3<f32> {
    if view_bindings::lights.n_directional_lights > 0u {
        let raw_color = view_bindings::lights.directional_lights[0u].color.rgb;
        let peak = max(max(raw_color.r, raw_color.g), raw_color.b);
        if peak > 0.0 {
            // Bevy stores illuminance in the GPU light color. Normalize it so
            // this small stylized model remains in a useful display range.
            return raw_color / peak;
        }
    }

    return vec3<f32>(1.0);
}

fn main_directional_shadow(input: VertexOutput, normal: vec3<f32>) -> f32 {
    if view_bindings::lights.n_directional_lights == 0u {
        return 1.0;
    }

    let light = view_bindings::lights.directional_lights[0u];
    if (light.flags & mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) == 0u {
        return 1.0;
    }

    let view_position = view_bindings::view.view_from_world * input.world_position;
    return shadows::fetch_directional_shadow(
        0u,
        input.world_position,
        normal,
        view_position.z,
        input.position.xy,
    );
}

@fragment
fn fragment(input: VertexOutput) -> @location(0) vec4<f32> {
    let controls = material.lanes[0];
    var base_tint = material.lanes[1];
#ifdef VERTEX_UVS_A
    base_tint *= textureSample(base_color_texture, base_color_sampler, input.uv);
#endif
    let roughness = clamp(controls.x, 0.0, 1.0);
    let rim_strength = clamp(controls.y, 0.0, 2.0);
    let highlight_strength = clamp(controls.z * 0.12, 0.0, 1.0);
    let bands = max(bitcast<u32>(controls.w), 1u);

    let normal = safe_normalize(input.world_normal, vec3<f32>(0.0, 1.0, 0.0));
    let view_direction = safe_normalize(
        view_bindings::view.world_position - input.world_position.xyz,
        vec3<f32>(0.0, 0.0, 1.0),
    );
    let light_direction = main_light_direction();
    let half_direction = safe_normalize(light_direction + view_direction, light_direction);

    let ndotl = max(dot(normal, light_direction), 0.0);
    let toon_band = floor(ndotl * f32(bands)) / f32(bands);
    let diffuse = mix(0.25, 1.0, toon_band);
    let shadow = main_directional_shadow(input, normal);
    let light_color = main_light_color();

    let ambient = max(
        min(view_bindings::lights.ambient_color.rgb * 0.22, vec3<f32>(0.22)),
        vec3<f32>(0.10),
    );
    let direct = diffuse * shadow * 0.86 * light_color;

    // Blinn-Phong specular response: rougher surfaces have a wider, weaker
    // highlight, while the toon diffuse term remains banded.
    let shininess = mix(128.0, 8.0, roughness);
    let specular = pow(max(dot(normal, half_direction), 0.0), shininess)
        * highlight_strength
        * mix(0.35, 1.0, shadow);

    let rim = pow(1.0 - max(dot(normal, view_direction), 0.0), 2.2)
        * rim_strength
        * 0.35
        * mix(0.60, 1.0, shadow);

    let final_rgb = base_tint.rgb * (ambient + direct)
        + vec3<f32>(specular + rim);
    return vec4<f32>(final_rgb, base_tint.a);
}
