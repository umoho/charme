use charme_shader::{ParameterValue, ShaderComposer, ShaderSource};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let controls = ShaderSource::new(
        r#"
#define_import_path controls

struct Controls {
    /// %{ ui.label = "Exposure"; ui.min = 0.0; ui.max = 8.0; }
    exposure: f32,

    /// %{ ui.color; ui.space = "linear"; }
    tint: vec3<f32>,
}

/// %{ reflect.parameters; ui.label = "Material controls"; }
@group(2) @binding(0)
var<uniform> controls: Controls;

fn color() -> vec4<f32> {
    return vec4<f32>(controls.tint * controls.exposure, 1.0);
}
"#,
        "controls.wgsl",
    );
    let shader = ShaderSource::new(
        r#"
#import controls

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return controls::color();
}
"#,
        "material.wgsl",
    );

    let mut composer = ShaderComposer::new();
    composer.add_composable_module(controls)?;
    let interface = composer.reflect(&shader)?;

    for block in &interface.parameter_blocks {
        println!(
            "{} @group({}) @binding({}), {} bytes",
            block.name, block.group, block.binding, block.size
        );
        for field in block.fields.iter().filter(|field| field.exposed) {
            println!(
                "  {}: {:?}, offset {}, metadata {:?}",
                field.name,
                field.ty,
                field.offset,
                field.metadata.as_ref().map(|metadata| &metadata.values)
            );
        }
    }

    let block = &interface.parameter_blocks[0];
    let mut buffer = block.create_buffer();
    buffer.set("exposure", ParameterValue::F32(1.5))?;
    buffer.set("tint", ParameterValue::F32Vector(vec![1.0, 0.8, 0.6]))?;
    println!("uniform bytes: {:?}", buffer.bytes());

    Ok(())
}
