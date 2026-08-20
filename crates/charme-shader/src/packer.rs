use charme_core::ParameterValue;

use crate::{ParameterBlock, ParameterField, ParameterType, ScalarType};

/// Mutable, zero-initialized host bytes for one reflected parameter block.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterBuffer {
    block: ParameterBlock,
    bytes: Vec<u8>,
}

impl ParameterBlock {
    pub fn create_buffer(&self) -> ParameterBuffer {
        ParameterBuffer {
            block: self.clone(),
            bytes: vec![0; self.size as usize],
        }
    }
}

impl ParameterBuffer {
    pub fn block(&self) -> &ParameterBlock {
        &self.block
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    /// Writes a value by its dot-separated reflected field path.
    pub fn set(&mut self, path: &str, value: ParameterValue) -> Result<(), ParameterWriteError> {
        let Some(field) = self
            .block
            .fields
            .iter()
            .find(|field| field.path.join(".") == path)
        else {
            return Err(ParameterWriteError::UnknownField {
                path: path.to_owned(),
            });
        };
        write_value(&mut self.bytes, field, path, value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterWriteError {
    UnknownField {
        path: String,
    },
    UnsupportedField {
        path: String,
        description: String,
    },
    TypeMismatch {
        path: String,
        expected: ParameterType,
        actual: &'static str,
    },
    OutOfBounds {
        path: String,
    },
}

impl std::fmt::Display for ParameterWriteError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownField { path } => write!(formatter, "unknown parameter field `{path}`"),
            Self::UnsupportedField { path, description } => {
                write!(
                    formatter,
                    "field `{path}` has unsupported type {description}"
                )
            }
            Self::TypeMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "field `{path}` expects {expected:?}, but received {actual}"
            ),
            Self::OutOfBounds { path } => {
                write!(
                    formatter,
                    "field `{path}` lies outside its parameter buffer"
                )
            }
        }
    }
}

impl std::error::Error for ParameterWriteError {}

fn write_value(
    bytes: &mut [u8],
    field: &ParameterField,
    path: &str,
    value: ParameterValue,
) -> Result<(), ParameterWriteError> {
    let encoded = match (&field.ty, value) {
        (ParameterType::Scalar(ScalarType::Bool), ParameterValue::Bool(value)) => {
            if value { 1u32 } else { 0u32 }.to_le_bytes().to_vec()
        }
        (ParameterType::Scalar(ScalarType::F32), ParameterValue::F32(value)) => {
            value.to_le_bytes().to_vec()
        }
        (ParameterType::Scalar(ScalarType::I32), ParameterValue::I32(value)) => {
            value.to_le_bytes().to_vec()
        }
        (ParameterType::Scalar(ScalarType::U32), ParameterValue::U32(value)) => {
            value.to_le_bytes().to_vec()
        }
        (
            ParameterType::Vector {
                scalar: ScalarType::F32,
                length,
            },
            ParameterValue::Vec2(values),
        ) => encode_vector(path, *length, ScalarType::F32, values, f32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::F32,
                length,
            },
            ParameterValue::Vec3(values),
        ) => encode_vector(path, *length, ScalarType::F32, values, f32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::F32,
                length,
            },
            ParameterValue::Vec4(values),
        ) => encode_vector(path, *length, ScalarType::F32, values, f32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::I32,
                length,
            },
            ParameterValue::IVec2(values),
        ) => encode_vector(path, *length, ScalarType::I32, values, i32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::I32,
                length,
            },
            ParameterValue::IVec3(values),
        ) => encode_vector(path, *length, ScalarType::I32, values, i32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::I32,
                length,
            },
            ParameterValue::IVec4(values),
        ) => encode_vector(path, *length, ScalarType::I32, values, i32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::U32,
                length,
            },
            ParameterValue::UVec2(values),
        ) => encode_vector(path, *length, ScalarType::U32, values, u32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::U32,
                length,
            },
            ParameterValue::UVec3(values),
        ) => encode_vector(path, *length, ScalarType::U32, values, u32::to_le_bytes)?,
        (
            ParameterType::Vector {
                scalar: ScalarType::U32,
                length,
            },
            ParameterValue::UVec4(values),
        ) => encode_vector(path, *length, ScalarType::U32, values, u32::to_le_bytes)?,
        (ParameterType::Unsupported { description }, _) => {
            return Err(ParameterWriteError::UnsupportedField {
                path: path.to_owned(),
                description: description.clone(),
            });
        }
        (expected, actual) => {
            return Err(ParameterWriteError::TypeMismatch {
                path: path.to_owned(),
                expected: expected.clone(),
                actual: kind_name(&actual),
            });
        }
    };

    let start = field.offset as usize;
    let Some(end) = start.checked_add(encoded.len()) else {
        return Err(ParameterWriteError::OutOfBounds {
            path: path.to_owned(),
        });
    };
    let Some(destination) = bytes.get_mut(start..end) else {
        return Err(ParameterWriteError::OutOfBounds {
            path: path.to_owned(),
        });
    };
    destination.copy_from_slice(&encoded);
    Ok(())
}

fn encode_vector<T: Copy, const WIDTH: usize, const LENGTH: usize>(
    path: &str,
    expected_length: u8,
    scalar: ScalarType,
    values: [T; LENGTH],
    encode: impl Fn(T) -> [u8; WIDTH],
) -> Result<Vec<u8>, ParameterWriteError> {
    if values.len() != expected_length as usize {
        return Err(ParameterWriteError::TypeMismatch {
            path: path.to_owned(),
            expected: ParameterType::Vector {
                scalar,
                length: expected_length,
            },
            actual: "vector with a different length",
        });
    }
    Ok(values
        .into_iter()
        .flat_map(|value| encode(value).into_iter())
        .collect())
}

fn kind_name(value: &ParameterValue) -> &'static str {
    match value {
        ParameterValue::Bool(_) => "bool",
        ParameterValue::F32(_) => "f32",
        ParameterValue::I32(_) => "i32",
        ParameterValue::U32(_) => "u32",
        ParameterValue::Vec2(_) | ParameterValue::Vec3(_) | ParameterValue::Vec4(_) => "f32 vector",
        ParameterValue::IVec2(_) | ParameterValue::IVec3(_) | ParameterValue::IVec4(_) => {
            "i32 vector"
        }
        ParameterValue::UVec2(_) | ParameterValue::UVec3(_) | ParameterValue::UVec4(_) => {
            "u32 vector"
        }
    }
}
