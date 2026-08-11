use serde::{Deserialize, Serialize};

/// A renderer-independent value stored by a Charme material instance.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum ParameterValue {
    /// A boolean value.
    Bool(bool),
    /// A signed 32-bit integer.
    I32(i32),
    /// An unsigned 32-bit integer.
    U32(u32),
    /// A 32-bit floating-point scalar.
    F32(f32),
    /// A two-component floating-point vector.
    Vec2([f32; 2]),
    /// A three-component floating-point vector.
    Vec3([f32; 3]),
    /// A four-component floating-point vector, also used for colors.
    Vec4([f32; 4]),
}

impl ParameterValue {
    /// Returns a stable name for the stored value type.
    pub const fn kind(&self) -> ParameterValueKind {
        match self {
            Self::Bool(_) => ParameterValueKind::Bool,
            Self::I32(_) => ParameterValueKind::I32,
            Self::U32(_) => ParameterValueKind::U32,
            Self::F32(_) => ParameterValueKind::F32,
            Self::Vec2(_) => ParameterValueKind::Vec2,
            Self::Vec3(_) => ParameterValueKind::Vec3,
            Self::Vec4(_) => ParameterValueKind::Vec4,
        }
    }

    /// Returns false when the value contains NaN or infinity.
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Bool(_) | Self::I32(_) | Self::U32(_) => true,
            Self::F32(value) => value.is_finite(),
            Self::Vec2(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec3(values) => values.iter().all(|value| value.is_finite()),
            Self::Vec4(values) => values.iter().all(|value| value.is_finite()),
        }
    }
}

/// The type of a [`ParameterValue`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ParameterValueKind {
    /// Boolean.
    Bool,
    /// Signed 32-bit integer.
    I32,
    /// Unsigned 32-bit integer.
    U32,
    /// 32-bit floating-point scalar.
    F32,
    /// Two-component floating-point vector.
    Vec2,
    /// Three-component floating-point vector.
    Vec3,
    /// Four-component floating-point vector.
    Vec4,
}
