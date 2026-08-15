//! Reusable Bevy 0.19 runtime support for materials authored with Charme.
//!
//! The first runtime ABI deliberately has a small, stable parameter block. It
//! is large enough for the editor preview while keeping the material usable by
//! ordinary Bevy applications without depending on the editor renderer.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use bevy::{
    asset::{Asset, Handle, load_internal_asset, uuid_handle},
    image::Image,
    math::Vec4,
    pbr::{Material, MaterialPlugin},
    prelude::AlphaMode,
    reflect::TypePath,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::{Shader, ShaderRef},
};
use charme_core::ParameterValue;
use thiserror::Error;

const CHARME_MATERIAL_SHADER: Handle<Shader> = uuid_handle!("c12bc8cb-4d38-47a3-8ef5-36a7a2b8d4f0");

/// Number of `vec4<f32>` lanes in the fixed material parameter ABI.
pub const CHARME_PARAMETER_LANES: usize = 16;
/// Number of bytes in the fixed material parameter ABI.
pub const CHARME_PARAMETER_BYTES: usize = CHARME_PARAMETER_LANES * 16;

/// The fixed, GPU-compatible parameter block used by Charme materials.
///
/// The first two lanes are currently assigned as follows:
/// `lane 0 = roughness, rim strength, highlight strength (legacy outline-width slot),
/// toon bands (bit cast)`; `lane 1 = base tint`. Remaining lanes are reserved for
/// future ABI fields.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct CharmeMaterialParams {
    /// Fixed-size lanes reserved for material parameters.
    pub lanes: [Vec4; CHARME_PARAMETER_LANES],
}

impl Default for CharmeMaterialParams {
    fn default() -> Self {
        Self::from_values(
            [0.45, 0.65, 1.25, f32::from_bits(3)],
            [0.82, 0.86, 1.0, 1.0],
        )
    }
}

impl CharmeMaterialParams {
    /// Creates the default ABI block with a caller-provided base tint.
    pub fn with_tint(tint: [f32; 4]) -> Self {
        Self::from_values([0.45, 0.65, 1.25, f32::from_bits(3)], tint)
    }

    /// Creates a block from the first control lane and tint lane.
    pub fn from_values(controls: [f32; 4], tint: [f32; 4]) -> Self {
        let mut lanes = [Vec4::ZERO; CHARME_PARAMETER_LANES];
        lanes[0] = Vec4::from_array(controls);
        lanes[1] = Vec4::from_array(tint);
        Self { lanes }
    }

    /// Returns the block as its fixed little-endian byte representation.
    pub fn to_bytes(self) -> [u8; CHARME_PARAMETER_BYTES] {
        let mut bytes = [0; CHARME_PARAMETER_BYTES];
        for (index, value) in self
            .lanes
            .iter()
            .flat_map(|lane| lane.to_array())
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_bits().to_le_bytes());
        }
        bytes
    }

    /// Reconstructs a block from the fixed ABI byte representation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ParameterBytesError> {
        if bytes.len() != CHARME_PARAMETER_BYTES {
            return Err(ParameterBytesError::WrongLength {
                actual: bytes.len(),
            });
        }
        let mut lanes = [Vec4::ZERO; CHARME_PARAMETER_LANES];
        for (lane_index, lane) in lanes.iter_mut().enumerate() {
            let mut values = [0.0; 4];
            for (component, value) in values.iter_mut().enumerate() {
                let start = (lane_index * 4 + component) * 4;
                *value = f32::from_bits(u32::from_le_bytes(
                    bytes[start..start + 4].try_into().expect("fixed ABI slice"),
                ));
            }
            *lane = Vec4::from_array(values);
        }
        Ok(Self { lanes })
    }

    /// Applies one of the named first-version ABI parameters.
    pub fn set_parameter(
        &mut self,
        path: &str,
        value: &ParameterValue,
    ) -> Result<(), ParameterError> {
        let field = path.strip_prefix("material.").unwrap_or(path);
        match (field, value) {
            ("roughness", ParameterValue::F32(value)) => self.lanes[0][0] = checked(*value)?,
            ("rim_strength", ParameterValue::F32(value)) => self.lanes[0][1] = checked(*value)?,
            // The third lane remains addressable as `outline_width` for document
            // compatibility, but currently controls the Blinn-Phong highlight.
            ("outline_width", ParameterValue::F32(value)) => self.lanes[0][2] = checked(*value)?,
            ("toon_bands", ParameterValue::U32(value)) => self.lanes[0][3] = f32::from_bits(*value),
            ("base_tint", ParameterValue::Vec4(value)) => {
                for component in value {
                    checked(*component)?;
                }
                self.lanes[1] = Vec4::from_array(*value);
            }
            ("roughness" | "rim_strength" | "outline_width", value) => {
                return Err(ParameterError::TypeMismatch {
                    path: path.to_owned(),
                    expected: "f32",
                    actual: kind_name(value),
                });
            }
            ("toon_bands", value) => {
                return Err(ParameterError::TypeMismatch {
                    path: path.to_owned(),
                    expected: "u32",
                    actual: kind_name(value),
                });
            }
            ("base_tint", value) => {
                return Err(ParameterError::TypeMismatch {
                    path: path.to_owned(),
                    expected: "vec4",
                    actual: kind_name(value),
                });
            }
            _ => {
                return Err(ParameterError::Unknown {
                    path: path.to_owned(),
                });
            }
        }
        Ok(())
    }
}

fn checked(value: f32) -> Result<f32, ParameterError> {
    value
        .is_finite()
        .then_some(value)
        .ok_or(ParameterError::NonFinite)
}

/// An error applying a value to the fixed Charme material ABI.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ParameterError {
    /// The path is not part of the first fixed ABI.
    #[error("unknown fixed Charme material parameter `{path}`")]
    Unknown {
        /// Parameter path that was not recognized.
        path: String,
    },
    /// The value has the wrong core type.
    #[error("parameter `{path}` expects {expected}, received {actual}")]
    TypeMismatch {
        /// Parameter path.
        path: String,
        /// Expected type.
        expected: &'static str,
        /// Actual type.
        actual: &'static str,
    },
    /// NaN and infinity are not accepted by the runtime ABI.
    #[error("material parameter contains a non-finite value")]
    NonFinite,
}

fn kind_name(value: &ParameterValue) -> &'static str {
    match value {
        ParameterValue::Bool(_) => "bool",
        ParameterValue::I32(_) => "i32",
        ParameterValue::U32(_) => "u32",
        ParameterValue::F32(_) => "f32",
        ParameterValue::Vec2(_) => "vec2",
        ParameterValue::Vec3(_) => "vec3",
        ParameterValue::Vec4(_) => "vec4",
    }
}

/// A reusable Charme material asset with the fixed parameter ABI.
#[derive(Asset, AsBindGroup, Clone, Debug, TypePath)]
pub struct CharmeMaterial {
    /// Values uploaded to the fixed material uniform buffer.
    #[uniform(0)]
    pub parameters: CharmeMaterialParams,
    /// Optional PMX diffuse/base-color texture.
    #[texture(1)]
    #[sampler(2)]
    #[dependency]
    pub base_color_texture: Option<Handle<Image>>,
    /// Alpha behavior for this material.
    pub alpha_mode: AlphaMode,
}

impl Default for CharmeMaterial {
    fn default() -> Self {
        Self {
            parameters: CharmeMaterialParams::default(),
            base_color_texture: None,
            alpha_mode: AlphaMode::Opaque,
        }
    }
}

impl CharmeMaterial {
    /// Creates an opaque material with a base tint.
    pub fn with_tint(tint: [f32; 4]) -> Self {
        Self {
            parameters: CharmeMaterialParams::with_tint(tint),
            ..Default::default()
        }
    }

    /// Applies a renderer-independent core value to this material.
    pub fn set_parameter(
        &mut self,
        path: &str,
        value: &ParameterValue,
    ) -> Result<(), ParameterError> {
        self.parameters.set_parameter(path, value)
    }
}

impl Material for CharmeMaterial {
    fn fragment_shader() -> ShaderRef {
        CHARME_MATERIAL_SHADER.clone().into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

/// Plugin that registers the reusable Charme material and its embedded shader.
#[derive(Clone, Copy, Debug, Default)]
pub struct CharmeMaterialPlugin;

impl bevy::app::Plugin for CharmeMaterialPlugin {
    fn build(&self, app: &mut bevy::app::App) {
        load_internal_asset!(
            app,
            CHARME_MATERIAL_SHADER,
            "charme_material.wgsl",
            Shader::from_wgsl
        );
        app.add_plugins(MaterialPlugin::<CharmeMaterial>::default());
    }
}

/// Error parsing a fixed ABI byte buffer.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ParameterBytesError {
    /// The supplied buffer is not exactly the ABI size.
    #[error("expected {CHARME_PARAMETER_BYTES} parameter bytes, got {actual}")]
    WrongLength {
        /// Actual number of supplied bytes.
        actual: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_abi_round_trips_and_preserves_integer_bits() {
        let mut params = CharmeMaterialParams::default();
        params
            .set_parameter("material.toon_bands", &ParameterValue::U32(7))
            .unwrap();
        let decoded = CharmeMaterialParams::from_bytes(&params.to_bytes()).unwrap();
        assert_eq!(decoded.lanes[0][3].to_bits(), 7);
    }

    #[test]
    fn fixed_abi_rejects_unknown_and_wrong_values() {
        let mut params = CharmeMaterialParams::default();
        assert!(matches!(
            params.set_parameter("material.unknown", &ParameterValue::F32(1.0)),
            Err(ParameterError::Unknown { .. })
        ));
        assert!(matches!(
            params.set_parameter("material.roughness", &ParameterValue::I32(1)),
            Err(ParameterError::TypeMismatch { .. })
        ));
    }
}
