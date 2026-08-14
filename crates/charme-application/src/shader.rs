use std::{collections::BTreeMap, path::PathBuf};

use charme_core::ParameterValue;
const BUILT_IN_SHADER: &str = include_str!("../../../assets/shaders/preview_material.wgsl");

use charme_shader::{
    MetadataBlock, MetadataValue, ParameterType, ScalarType, ShaderComposer, ShaderInterface,
    ShaderSource,
};

/// The native editor control family selected for one reflected parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterControlKind {
    /// A continuous floating-point slider.
    Float,
    /// A rounded signed integer slider.
    SignedInteger,
    /// A rounded non-negative unsigned integer slider.
    UnsignedInteger,
}

/// Presentation metadata for one reflected shader parameter.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterControlSpec {
    /// Stable parameter path sent back in an edit action.
    pub key: String,
    /// User-facing label.
    pub label: String,
    /// Minimum control value.
    pub minimum: f64,
    /// Maximum control value.
    pub maximum: f64,
    /// Initial control value.
    pub initial: f64,
    /// Native control family.
    pub kind: ParameterControlKind,
}

/// Presentation projection of a reflected shader interface.
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderInspection {
    /// Inspected shader path.
    pub path: PathBuf,
    /// Native controls that can be created for exposed scalar fields.
    pub controls: Vec<ParameterControlSpec>,
    /// Number of reflected parameter blocks.
    pub parameter_block_count: usize,
    /// Number of exposed fields that do not currently have a native control.
    pub non_scalar_field_count: usize,
    /// Human-readable reflected diagnostics.
    pub diagnostics: Vec<String>,
}

/// Reflects WGSL source and builds a UI-independent inspection projection.
pub fn inspect_shader_source(path: PathBuf, source: &str) -> Result<ShaderInspection, String> {
    ShaderComposer::new()
        .reflect(&ShaderSource::new(source, path.to_string_lossy()))
        .map(|interface| build_inspection(path.clone(), &interface))
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

/// Reflects the built-in preview shader used for PMX material slots.
pub fn inspect_preview_shader() -> Result<ShaderInspection, String> {
    inspect_shader_source(
        PathBuf::from("assets/shaders/preview_material.wgsl"),
        BUILT_IN_SHADER,
    )
}

/// Applies persisted material values to reflected scalar control defaults.
pub fn controls_for_material(
    inspection: &ShaderInspection,
    parameters: &BTreeMap<String, ParameterValue>,
) -> Vec<ParameterControlSpec> {
    inspection
        .controls
        .iter()
        .cloned()
        .map(|mut control| {
            if let Some(value) = parameters.get(&control.key)
                && let Some(number) = parameter_number(value)
            {
                control.initial = number.clamp(control.minimum, control.maximum);
            }
            control
        })
        .collect()
}

fn parameter_number(value: &ParameterValue) -> Option<f64> {
    match value {
        ParameterValue::F32(value) => Some(*value as f64),
        ParameterValue::I32(value) => Some(*value as f64),
        ParameterValue::U32(value) => Some(*value as f64),
        ParameterValue::Bool(_)
        | ParameterValue::Vec2(_)
        | ParameterValue::Vec3(_)
        | ParameterValue::Vec4(_) => None,
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

fn metadata_number(metadata: &MetadataBlock, path: &str) -> Option<f64> {
    match metadata.get(path)? {
        MetadataValue::Integer(value) => Some(*value as f64),
        MetadataValue::Float(value) => Some(*value),
        MetadataValue::Bool(_) | MetadataValue::String(_) | MetadataValue::Identifier(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUILT_IN_SHADER: &str = include_str!("../../../assets/shaders/preview_material.wgsl");

    #[test]
    fn built_in_shader_produces_native_scalar_controls() {
        let inspection =
            inspect_shader_source(PathBuf::from("built-in.wgsl"), BUILT_IN_SHADER).unwrap();

        assert_eq!(inspection.parameter_block_count, 1);
        assert_eq!(inspection.controls.len(), 4);
        assert_eq!(inspection.controls[0].label, "Roughness");
        assert_eq!(inspection.controls[0].initial, 0.45);
        assert_eq!(inspection.non_scalar_field_count, 1);
    }

    #[test]
    fn reflected_controls_use_the_selected_material_value() {
        let inspection = inspect_preview_shader().unwrap();
        let parameters =
            BTreeMap::from([("material.roughness".to_owned(), ParameterValue::F32(0.8))]);
        let controls = controls_for_material(&inspection, &parameters);

        assert_eq!(controls[0].key, "material.roughness");
        assert!((controls[0].initial - 0.8).abs() < 1e-6);
    }
}
