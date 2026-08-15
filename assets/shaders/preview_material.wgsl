struct PreviewMaterialParams {
    /// Base surface roughness.
    /// %{ ui.label = "粗糙度"; ui.min = 0.0; ui.max = 1.0; ui.default = 0.45; }
    roughness: f32,

    /// Strength of the view-dependent rim light.
    /// %{ ui.label = "边缘光强度"; ui.min = 0.0; ui.max = 2.0; ui.default = 0.65; }
    rim_strength: f32,

    /// Compatibility slot used for the specular highlight until an outline pass exists.
    /// %{ ui.label = "高光强度"; ui.min = 0.0; ui.max = 10.0; ui.default = 1.25; }
    outline_width: f32,

    /// Number of toon-lighting bands.
    /// %{ ui.label = "卡通分段数"; ui.min = 1; ui.max = 8; ui.default = 3; }
    toon_bands: u32,

    /// %{ ui.label = "基础颜色"; ui.color; ui.space = "linear"; }
    base_tint: vec4<f32>,
}

/// %{ reflect.parameters; ui.label = "角色预览材质"; }
@group(2) @binding(0)
var<uniform> material: PreviewMaterialParams;

struct PreviewFragmentInput {
    @location(0) normal: vec3<f32>,
    @location(1) view_direction: vec3<f32>,
}

fn safe_normalize(value: vec3<f32>, fallback: vec3<f32>) -> vec3<f32> {
    let magnitude = length(value);
    if magnitude > 0.0001 {
        return value / magnitude;
    }
    return fallback;
}

fn preview_lighting(normal: vec3<f32>, view_direction: vec3<f32>) -> vec3<f32> {
    let n = safe_normalize(normal, vec3<f32>(0.0, 1.0, 0.0));
    let v = safe_normalize(view_direction, vec3<f32>(0.0, 0.0, 1.0));
    let l = safe_normalize(vec3<f32>(-0.35, 0.8, 0.45), vec3<f32>(0.0, 1.0, 0.0));
    let h = safe_normalize(l + v, l);

    let roughness = clamp(material.roughness, 0.0, 1.0);
    let bands = max(material.toon_bands, 1u);
    let ndotl = max(dot(n, l), 0.0);
    let toon_band = floor(ndotl * f32(bands)) / f32(bands);
    let diffuse = mix(0.25, 1.0, toon_band);

    let shininess = mix(128.0, 8.0, roughness);
    let specular = pow(max(dot(n, h), 0.0), shininess)
        * clamp(material.outline_width * 0.12, 0.0, 1.0);

    let rim = pow(1.0 - max(dot(n, v), 0.0), 2.2)
        * clamp(material.rim_strength, 0.0, 2.0)
        * 0.35;

    let ambient = 0.12;
    return vec3<f32>(ambient + diffuse * 0.86) + vec3<f32>(specular + rim);
}

@fragment
fn fragment(input: PreviewFragmentInput) -> @location(0) vec4<f32> {
    return vec4<f32>(
        material.base_tint.rgb * preview_lighting(input.normal, input.view_direction),
        material.base_tint.a,
    );
}
