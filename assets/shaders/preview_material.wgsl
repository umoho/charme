struct PreviewMaterialParams {
    /// Base surface roughness.
    /// %{ ui.label = "Roughness"; ui.min = 0.0; ui.max = 1.0; ui.default = 0.45; }
    roughness: f32,

    /// Strength of the view-dependent rim light.
    /// %{ ui.label = "Rim Strength"; ui.min = 0.0; ui.max = 2.0; ui.default = 0.65; }
    rim_strength: f32,

    /// Width of the character outline.
    /// %{ ui.label = "Outline Width"; ui.min = 0.0; ui.max = 10.0; ui.default = 1.25; }
    outline_width: f32,

    /// Number of toon-lighting bands.
    /// %{ ui.label = "Toon Bands"; ui.min = 1; ui.max = 8; ui.default = 3; }
    toon_bands: u32,

    /// %{ ui.label = "Base Tint"; ui.color; ui.space = "linear"; }
    base_tint: vec4<f32>,
}

/// %{ reflect.parameters; ui.label = "Character Preview"; }
@group(2) @binding(0)
var<uniform> material: PreviewMaterialParams;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    let bands = max(material.toon_bands, 1u);
    let band = floor((0.35 + material.rim_strength * 0.2) * f32(bands)) / f32(bands);
    let intensity = band * (1.0 - material.roughness * 0.35)
        + material.outline_width * 0.02;
    return vec4<f32>(material.base_tint.rgb * intensity, material.base_tint.a);
}
