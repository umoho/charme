use std::{fs, path::PathBuf, thread};

use cacao::appkit::App;
use charme_shader::{
    MetadataValue, ParameterType, ScalarType, ShaderComposer, ShaderInterface, ShaderSource,
};

use crate::app::{CharmeApp, Message};

const BUILT_IN_SHADER: &str = include_str!("../../../assets/shaders/preview_material.wgsl");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParameterControlKind {
    Float,
    SignedInteger,
    UnsignedInteger,
}

#[derive(Clone, Debug)]
pub(crate) struct ParameterControlSpec {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) minimum: f64,
    pub(crate) maximum: f64,
    pub(crate) initial: f64,
    pub(crate) kind: ParameterControlKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ShaderInspection {
    pub(crate) path: PathBuf,
    pub(crate) controls: Vec<ParameterControlSpec>,
    pub(crate) parameter_block_count: usize,
    pub(crate) non_scalar_field_count: usize,
    pub(crate) diagnostics: Vec<String>,
}

pub(crate) fn inspect_built_in_shader() {
    inspect_source(
        PathBuf::from("Built-in Character Preview"),
        BUILT_IN_SHADER.to_owned(),
    );
}

pub(crate) fn inspect_shader(path: PathBuf) {
    thread::Builder::new()
        .name("charme-shader-inspection".to_owned())
        .spawn(move || {
            let result = fs::read_to_string(&path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))
                .and_then(|source| reflect_source(path.clone(), source));
            App::<CharmeApp, Message>::dispatch_main(Message::ShaderInspected { path, result });
        })
        .expect("failed to start shader inspection worker");
}

fn inspect_source(path: PathBuf, source: String) {
    thread::Builder::new()
        .name("charme-shader-inspection".to_owned())
        .spawn(move || {
            let result = reflect_source(path.clone(), source);
            App::<CharmeApp, Message>::dispatch_main(Message::ShaderInspected { path, result });
        })
        .expect("failed to start shader inspection worker");
}

fn reflect_source(path: PathBuf, source: String) -> Result<ShaderInspection, String> {
    ShaderComposer::new()
        .reflect(&ShaderSource::new(source, path.to_string_lossy()))
        .map(|interface| build_inspection(path, &interface))
        .map_err(|error| error.to_string())
}

fn build_inspection(path: PathBuf, interface: &ShaderInterface) -> ShaderInspection {
    let mut controls = Vec::new();
    let mut non_scalar_field_count = 0;
    for block in &interface.parameter_blocks {
        for field in block.fields.iter().filter(|field| field.exposed) {
            let Some((kind, default_minimum, default_maximum)) = control_kind(&field.ty) else {
                non_scalar_field_count += 1;
                continue;
            };
            let metadata = field.metadata.as_ref();
            let minimum = metadata
                .and_then(|metadata| metadata_number(&metadata.values, "ui.min"))
                .unwrap_or(default_minimum);
            let maximum = metadata
                .and_then(|metadata| metadata_number(&metadata.values, "ui.max"))
                .unwrap_or(default_maximum);
            let (minimum, maximum) = if minimum < maximum {
                (minimum, maximum)
            } else {
                (default_minimum, default_maximum)
            };
            let initial = metadata
                .and_then(|metadata| metadata_number(&metadata.values, "ui.default"))
                .unwrap_or(0.0)
                .clamp(minimum, maximum);
            let label = metadata
                .and_then(|metadata| metadata.values.get("ui.label"))
                .and_then(|value| match value {
                    MetadataValue::String(value) => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| field.name.clone());
            controls.push(ParameterControlSpec {
                key: format!("{}.{}", block.name, field.path.join(".")),
                label,
                minimum,
                maximum,
                initial,
                kind,
            });
        }
    }
    ShaderInspection {
        path,
        controls,
        parameter_block_count: interface.parameter_blocks.len(),
        non_scalar_field_count,
        diagnostics: interface
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{:?}", diagnostic.kind))
            .collect(),
    }
}

fn control_kind(parameter_type: &ParameterType) -> Option<(ParameterControlKind, f64, f64)> {
    match parameter_type {
        ParameterType::Scalar(ScalarType::F32) => Some((ParameterControlKind::Float, 0.0, 1.0)),
        ParameterType::Scalar(ScalarType::I32) => {
            Some((ParameterControlKind::SignedInteger, -100.0, 100.0))
        }
        ParameterType::Scalar(ScalarType::U32) => {
            Some((ParameterControlKind::UnsignedInteger, 0.0, 100.0))
        }
        ParameterType::Vector { .. } | ParameterType::Unsupported { .. } => None,
    }
}

fn metadata_number(metadata: &charme_shader::MetadataBlock, path: &str) -> Option<f64> {
    match metadata.get(path)? {
        MetadataValue::Integer(value) => Some(*value as f64),
        MetadataValue::Float(value) => Some(*value),
        MetadataValue::Bool(_) | MetadataValue::String(_) | MetadataValue::Identifier(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_shader_produces_native_scalar_controls() {
        let interface = ShaderComposer::new()
            .reflect(&ShaderSource::new(BUILT_IN_SHADER, "built-in.wgsl"))
            .unwrap();
        let inspection = build_inspection(PathBuf::from("built-in.wgsl"), &interface);

        assert_eq!(inspection.parameter_block_count, 1);
        assert_eq!(inspection.controls.len(), 4);
        assert_eq!(inspection.controls[0].label, "Roughness");
        assert_eq!(inspection.controls[0].initial, 0.45);
        assert_eq!(inspection.non_scalar_field_count, 1);
    }
}
