use charme_shader::{
    InterfaceDiagnosticKind, MetadataValue, ParameterType, ParameterValue, ResourceKind,
    ScalarType, ShaderComposer, ShaderDefValue, ShaderSource, TextureClass, TextureDimension,
};

#[test]
fn reflects_and_packs_a_marked_uniform_block() {
    let source = ShaderSource::new(
        r#"
struct Params {
    /// Surface roughness.
    /// %{ ui.min = 0.0; ui.max = 1.0; }
    roughness: f32,

    /// %{ ui.color; }
    tint: vec3<f32>,

    /// %{ ui.min = 0; ui.max = 3; }
    mode: u32,
}

/// Editable material parameters.
/// %{ reflect.parameters; ui.label = "Material"; }
@group(2) @binding(0)
var<uniform> params: Params;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(params.tint, params.roughness + f32(params.mode));
}
"#,
        "material.wgsl",
    );

    let interface = ShaderComposer::new().reflect(&source).unwrap();
    assert!(
        interface.diagnostics.is_empty(),
        "{:?}",
        interface.diagnostics
    );
    assert_eq!(interface.entry_points.len(), 1);
    assert_eq!(interface.resources.len(), 1);
    assert_eq!(interface.resources[0].kind, ResourceKind::UniformBuffer);
    assert_eq!(interface.resources[0].used_by.len(), 1);

    let block = &interface.parameter_blocks[0];
    assert_eq!((block.group, block.binding), (2, 0));
    assert_eq!(block.size, 32);
    assert_eq!(block.alignment, 16);
    assert_eq!(
        block.metadata.values.get("ui.label"),
        Some(&MetadataValue::String("Material".to_owned()))
    );

    assert_eq!(block.fields[0].offset, 0);
    assert_eq!(block.fields[0].ty, ParameterType::Scalar(ScalarType::F32));
    assert_eq!(block.fields[1].offset, 16);
    assert_eq!(
        block.fields[1].ty,
        ParameterType::Vector {
            scalar: ScalarType::F32,
            length: 3,
        }
    );
    assert_eq!(block.fields[2].offset, 28);

    let mut buffer = block.create_buffer();
    buffer.set("roughness", ParameterValue::F32(0.25)).unwrap();
    buffer
        .set("tint", ParameterValue::Vec3([1.0, 0.5, 0.25]))
        .unwrap();
    buffer.set("mode", ParameterValue::U32(2)).unwrap();

    assert_eq!(&buffer.bytes()[0..4], &0.25_f32.to_le_bytes());
    assert!(buffer.bytes()[4..16].iter().all(|byte| *byte == 0));
    assert_eq!(&buffer.bytes()[16..20], &1.0_f32.to_le_bytes());
    assert_eq!(&buffer.bytes()[20..24], &0.5_f32.to_le_bytes());
    assert_eq!(&buffer.bytes()[24..28], &0.25_f32.to_le_bytes());
    assert_eq!(&buffer.bytes()[28..32], &2_u32.to_le_bytes());
}

#[test]
fn carries_metadata_from_an_imported_module() {
    let controls = ShaderSource::new(
        r#"
#define_import_path controls

struct ControlParams {
    /// %{ ui.min = 0.0; ui.max = 4.0; }
    gain: f32,
}

/// %{ reflect.parameters; ui.label = "Controls"; }
@group(1) @binding(0)
var<uniform> control_params: ControlParams;

fn apply_gain(value: f32) -> f32 {
    return value * control_params.gain;
}
"#,
        "controls.wgsl",
    );
    let root = ShaderSource::new(
        r#"
#import controls

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(controls::apply_gain(0.5));
}
"#,
        "root.wgsl",
    );

    let mut composer = ShaderComposer::new();
    assert_eq!(
        composer.add_composable_module(controls).unwrap(),
        "controls"
    );
    let interface = composer.reflect(&root).unwrap();

    assert!(
        interface.diagnostics.is_empty(),
        "{:?}",
        interface.diagnostics
    );
    let block = interface
        .parameter_blocks
        .iter()
        .find(|block| block.group == 1 && block.binding == 0)
        .unwrap();
    assert_eq!(block.name, "control_params");
    assert_eq!(block.metadata.module, "controls");
    assert_eq!(
        block.metadata.values.get("ui.label"),
        Some(&MetadataValue::String("Controls".to_owned()))
    );
    assert_eq!(
        block.fields[0].metadata.as_ref().unwrap().module,
        "controls"
    );
    assert_eq!(
        block.fields[0]
            .metadata
            .as_ref()
            .unwrap()
            .values
            .get("ui.max"),
        Some(&MetadataValue::Float(4.0))
    );
}

#[test]
fn reflects_the_selected_shader_def_variant() {
    let source = r#"
struct Params {
    base: f32,
#ifdef EXTRA
    /// %{ ui.label = "Extra"; }
    extra: vec2<f32>,
#endif
}

/// %{ reflect.parameters; }
@group(0) @binding(0)
var<uniform> params: Params;

@fragment
fn fragment() -> @location(0) vec4<f32> {
#ifdef EXTRA
    return vec4<f32>(params.base, params.extra, 1.0);
#else
    return vec4<f32>(params.base);
#endif
}
"#;

    let without_extra = ShaderComposer::new()
        .reflect(&ShaderSource::new(source, "variant.wgsl"))
        .unwrap();
    assert_eq!(without_extra.parameter_blocks[0].fields.len(), 1);

    let with_extra = ShaderComposer::new()
        .reflect(
            &ShaderSource::new(source, "variant.wgsl")
                .with_shader_def("EXTRA", ShaderDefValue::Bool(true)),
        )
        .unwrap();
    assert_eq!(with_extra.parameter_blocks[0].fields.len(), 2);
    assert_eq!(with_extra.parameter_blocks[0].fields[1].name, "extra");
    assert_eq!(
        with_extra.parameter_blocks[0].fields[1]
            .metadata
            .as_ref()
            .unwrap()
            .values
            .get("ui.label"),
        Some(&MetadataValue::String("Extra".to_owned()))
    );
}

#[test]
fn reflects_texture_resources_and_entry_point_usage() {
    let source = ShaderSource::new(
        r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, vec2<f32>(0.5));
}
"#,
        "texture.wgsl",
    );

    let interface = ShaderComposer::new().reflect(&source).unwrap();
    assert_eq!(interface.resources.len(), 2);
    assert_eq!(
        interface.resources[0].kind,
        ResourceKind::Texture {
            dimension: TextureDimension::D2,
            arrayed: false,
            multisampled: false,
            class: TextureClass::Sampled,
        }
    );
    assert_eq!(
        interface.resources[1].kind,
        ResourceKind::Sampler { comparison: false }
    );
    assert!(
        interface
            .resources
            .iter()
            .all(|resource| resource.used_by.len() == 1 && resource.used_by[0].read)
    );
}

#[test]
fn keeps_unsupported_fields_visible_with_diagnostics() {
    let source = ShaderSource::new(
        r#"
struct Params { transform: mat4x4<f32> }
/// %{ reflect.parameters; }
@group(0) @binding(0) var<uniform> params: Params;
@vertex fn vertex() -> @builtin(position) vec4<f32> {
    return params.transform * vec4<f32>(0.0, 0.0, 0.0, 1.0);
}
"#,
        "matrix.wgsl",
    );

    let interface = ShaderComposer::new().reflect(&source).unwrap();
    assert!(matches!(
        interface.parameter_blocks[0].fields[0].ty,
        ParameterType::Unsupported { .. }
    ));
    assert!(interface.diagnostics.iter().any(|diagnostic| matches!(
        diagnostic.kind,
        InterfaceDiagnosticKind::UnsupportedParameterType { .. }
    )));
}

#[test]
fn reports_a_type_mismatch_without_corrupting_the_buffer() {
    let source = ShaderSource::new(
        r#"
struct Params { value: f32 }
/// %{ reflect.parameters; }
@group(0) @binding(0) var<uniform> params: Params;
@fragment fn fragment() -> @location(0) vec4<f32> { return vec4(params.value); }
"#,
        "mismatch.wgsl",
    );
    let interface = ShaderComposer::new().reflect(&source).unwrap();
    let mut buffer = interface.parameter_blocks[0].create_buffer();

    assert!(buffer.set("value", ParameterValue::U32(1)).is_err());
    assert!(buffer.bytes().iter().all(|byte| *byte == 0));
}
