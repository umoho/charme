use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{DocumentId, MaterialId, MaterialSlotId, ParameterValue, ResourcePath, ShaderId};

/// Schema version written by this release.
pub const CURRENT_DOCUMENT_VERSION: u32 = 1;

/// A complete, serializable Charme project document.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CharmeDocument {
    pub(crate) version: u32,
    pub(crate) id: DocumentId,
    pub(crate) name: String,
    pub(crate) character: Option<CharacterSource>,
    pub(crate) shaders: Vec<ShaderSource>,
    pub(crate) materials: Vec<MaterialInstance>,
    pub(crate) material_slots: Vec<MaterialSlot>,
}

impl CharmeDocument {
    /// Creates an empty document using the current schema version.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: CURRENT_DOCUMENT_VERSION,
            id: DocumentId::new(),
            name: name.into(),
            character: None,
            shaders: Vec::new(),
            materials: Vec::new(),
            material_slots: Vec::new(),
        }
    }

    /// Returns the serialized schema version.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the stable document identifier.
    pub const fn id(&self) -> DocumentId {
        self.id
    }

    /// Returns the display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the character source, if one has been selected.
    pub const fn character(&self) -> Option<&CharacterSource> {
        self.character.as_ref()
    }

    /// Returns shader sources in document order.
    pub fn shaders(&self) -> &[ShaderSource] {
        &self.shaders
    }

    /// Returns material instances in document order.
    pub fn materials(&self) -> &[MaterialInstance] {
        &self.materials
    }

    /// Returns imported character material slots in source order.
    pub fn material_slots(&self) -> &[MaterialSlot] {
        &self.material_slots
    }

    /// Finds a shader by stable identifier.
    pub fn shader(&self, id: ShaderId) -> Option<&ShaderSource> {
        self.shaders.iter().find(|shader| shader.id == id)
    }

    /// Finds a material by stable identifier.
    pub fn material(&self, id: MaterialId) -> Option<&MaterialInstance> {
        self.materials.iter().find(|material| material.id == id)
    }

    /// Finds an imported material slot by stable identifier.
    pub fn material_slot(&self, id: MaterialSlotId) -> Option<&MaterialSlot> {
        self.material_slots.iter().find(|slot| slot.id == id)
    }

    /// Validates references, IDs and serializable numeric values.
    pub fn validate(&self) -> Result<(), DocumentValidationError> {
        if self.version != CURRENT_DOCUMENT_VERSION {
            return Err(DocumentValidationError::UnsupportedVersion(self.version));
        }

        ensure_unique(
            self.shaders.iter().map(|shader| shader.id),
            DocumentValidationError::DuplicateShader,
        )?;
        ensure_unique(
            self.materials.iter().map(|material| material.id),
            DocumentValidationError::DuplicateMaterial,
        )?;
        ensure_unique(
            self.material_slots.iter().map(|slot| slot.id),
            DocumentValidationError::DuplicateMaterialSlot,
        )?;
        ensure_unique(
            self.material_slots.iter().map(|slot| slot.source_index),
            DocumentValidationError::DuplicateSourceMaterialIndex,
        )?;

        let shader_ids = self
            .shaders
            .iter()
            .map(|shader| shader.id)
            .collect::<HashSet<_>>();
        let material_ids = self
            .materials
            .iter()
            .map(|material| material.id)
            .collect::<HashSet<_>>();

        for material in &self.materials {
            if !shader_ids.contains(&material.shader) {
                return Err(DocumentValidationError::MissingShader {
                    material: material.id,
                    shader: material.shader,
                });
            }
            for (path, value) in &material.parameters {
                if path.trim().is_empty() {
                    return Err(DocumentValidationError::EmptyParameterPath(material.id));
                }
                if !value.is_finite() {
                    return Err(DocumentValidationError::NonFiniteParameter {
                        material: material.id,
                        path: path.clone(),
                    });
                }
            }
            for path in material.textures.keys() {
                if path.trim().is_empty() {
                    return Err(DocumentValidationError::EmptyTexturePath(material.id));
                }
            }
            if let MaterialAlphaMode::Mask { cutoff } = material.render_state.alpha_mode
                && (!cutoff.is_finite() || !(0.0..=1.0).contains(&cutoff))
            {
                return Err(DocumentValidationError::InvalidAlphaCutoff {
                    material: material.id,
                    cutoff,
                });
            }
        }

        for slot in &self.material_slots {
            if let Some(material) = slot.material
                && !material_ids.contains(&material)
            {
                return Err(DocumentValidationError::MissingBoundMaterial {
                    slot: slot.id,
                    material,
                });
            }
        }
        Ok(())
    }
}

fn ensure_unique<T: Copy + Eq + std::hash::Hash>(
    values: impl IntoIterator<Item = T>,
    error: impl Fn(T) -> DocumentValidationError,
) -> Result<(), DocumentValidationError> {
    let mut found = HashSet::new();
    for value in values {
        if !found.insert(value) {
            return Err(error(value));
        }
    }
    Ok(())
}

/// A character model referenced by a document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CharacterSource {
    /// Character file path, or the ZIP archive containing the PMX file.
    pub path: ResourcePath,
    /// Character asset format.
    pub format: CharacterFormat,
    /// Archive-relative PMX entry when `path` points to a ZIP archive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_entry: Option<String>,
}

impl CharacterSource {
    /// Creates a PMX character source.
    pub fn pmx(path: ResourcePath) -> Self {
        Self {
            path,
            format: CharacterFormat::Pmx,
            archive_entry: None,
        }
    }

    /// Creates a PMX character source whose bytes are stored in a ZIP archive.
    pub fn pmx_with_archive_entry(path: ResourcePath, archive_entry: impl Into<String>) -> Self {
        Self {
            path,
            format: CharacterFormat::Pmx,
            archive_entry: Some(archive_entry.into()),
        }
    }

    /// Returns the selected PMX entry inside an archive, if any.
    pub fn archive_entry(&self) -> Option<&str> {
        self.archive_entry.as_deref()
    }
}

/// Supported character source formats.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CharacterFormat {
    /// MikuMikuDance PMX.
    Pmx,
}

/// One WGSL root source known to a document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShaderSource {
    pub(crate) id: ShaderId,
    pub(crate) name: String,
    pub(crate) path: ResourcePath,
}

impl ShaderSource {
    /// Creates a shader reference with a new stable identifier.
    pub fn new(name: impl Into<String>, path: ResourcePath) -> Self {
        Self {
            id: ShaderId::new(),
            name: name.into(),
            path,
        }
    }

    /// Returns the stable shader identifier.
    pub const fn id(&self) -> ShaderId {
        self.id
    }

    /// Returns the display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the WGSL source path.
    pub const fn path(&self) -> &ResourcePath {
        &self.path
    }
}

/// An editable material instance backed by a WGSL shader.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MaterialInstance {
    pub(crate) id: MaterialId,
    pub(crate) name: String,
    pub(crate) shader: ShaderId,
    pub(crate) parameters: BTreeMap<String, ParameterValue>,
    pub(crate) textures: BTreeMap<String, ResourcePath>,
    pub(crate) render_state: MaterialRenderState,
}

impl MaterialInstance {
    /// Creates an empty material instance with a new stable identifier.
    pub fn new(name: impl Into<String>, shader: ShaderId) -> Self {
        Self {
            id: MaterialId::new(),
            name: name.into(),
            shader,
            parameters: BTreeMap::new(),
            textures: BTreeMap::new(),
            render_state: MaterialRenderState::default(),
        }
    }

    /// Returns the stable material identifier.
    pub const fn id(&self) -> MaterialId {
        self.id
    }

    /// Returns the display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the shader used by this instance.
    pub const fn shader(&self) -> ShaderId {
        self.shader
    }

    /// Returns reflected parameter values by field path.
    pub const fn parameters(&self) -> &BTreeMap<String, ParameterValue> {
        &self.parameters
    }

    /// Returns texture references by reflected resource path.
    pub const fn textures(&self) -> &BTreeMap<String, ResourcePath> {
        &self.textures
    }

    /// Returns rasterization and blending settings.
    pub const fn render_state(&self) -> MaterialRenderState {
        self.render_state
    }
}

/// Render state stored with a material instance.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct MaterialRenderState {
    /// Alpha rendering mode.
    pub alpha_mode: MaterialAlphaMode,
    /// Whether both triangle sides should be visible.
    pub double_sided: bool,
}

impl Default for MaterialRenderState {
    fn default() -> Self {
        Self {
            alpha_mode: MaterialAlphaMode::Opaque,
            double_sided: false,
        }
    }
}

/// Material alpha rendering behavior.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum MaterialAlphaMode {
    /// Fully opaque rendering.
    Opaque,
    /// Binary alpha test using `cutoff`.
    Mask {
        /// Alpha values below this value are discarded.
        cutoff: f32,
    },
    /// Alpha-blended rendering.
    Blend,
}

/// One material slot imported from the character source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MaterialSlot {
    pub(crate) id: MaterialSlotId,
    pub(crate) source_index: u32,
    pub(crate) source_name: String,
    pub(crate) source_english_name: String,
    pub(crate) material: Option<MaterialId>,
}

impl MaterialSlot {
    /// Creates an unbound imported material slot.
    pub fn new(
        source_index: u32,
        source_name: impl Into<String>,
        source_english_name: impl Into<String>,
    ) -> Self {
        Self {
            id: MaterialSlotId::new(),
            source_index,
            source_name: source_name.into(),
            source_english_name: source_english_name.into(),
            material: None,
        }
    }

    /// Creates an imported material slot with a caller-provided stable identifier.
    pub fn with_id(
        id: MaterialSlotId,
        source_index: u32,
        source_name: impl Into<String>,
        source_english_name: impl Into<String>,
        material: Option<MaterialId>,
    ) -> Self {
        Self {
            id,
            source_index,
            source_name: source_name.into(),
            source_english_name: source_english_name.into(),
            material,
        }
    }

    /// Returns the stable slot identifier.
    pub const fn id(&self) -> MaterialSlotId {
        self.id
    }

    /// Returns the material index in the source PMX document.
    pub const fn source_index(&self) -> u32 {
        self.source_index
    }

    /// Returns the source material name.
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Returns the source English material name.
    pub fn source_english_name(&self) -> &str {
        &self.source_english_name
    }

    /// Returns the assigned Charme material, if any.
    pub const fn material(&self) -> Option<MaterialId> {
        self.material
    }
}

/// A structurally invalid or unsupported document.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum DocumentValidationError {
    /// The document schema is newer or older than this implementation supports.
    #[error("unsupported Charme document version {0}")]
    UnsupportedVersion(u32),
    /// A shader ID occurs more than once.
    #[error("duplicate shader ID {0}")]
    DuplicateShader(ShaderId),
    /// A material ID occurs more than once.
    #[error("duplicate material ID {0}")]
    DuplicateMaterial(MaterialId),
    /// A slot ID occurs more than once.
    #[error("duplicate material slot ID {0}")]
    DuplicateMaterialSlot(MaterialSlotId),
    /// A PMX source material index occurs more than once.
    #[error("duplicate source material index {0}")]
    DuplicateSourceMaterialIndex(u32),
    /// A material references an unknown shader.
    #[error("material {material} references missing shader {shader}")]
    MissingShader {
        /// Material containing the reference.
        material: MaterialId,
        /// Missing shader.
        shader: ShaderId,
    },
    /// A slot references an unknown material.
    #[error("slot {slot} references missing material {material}")]
    MissingBoundMaterial {
        /// Slot containing the reference.
        slot: MaterialSlotId,
        /// Missing material.
        material: MaterialId,
    },
    /// A parameter has no field path.
    #[error("material {0} contains an empty parameter path")]
    EmptyParameterPath(MaterialId),
    /// A texture has no resource path.
    #[error("material {0} contains an empty texture binding path")]
    EmptyTexturePath(MaterialId),
    /// A parameter contains NaN or infinity.
    #[error("material {material} parameter {path} is not finite")]
    NonFiniteParameter {
        /// Material containing the value.
        material: MaterialId,
        /// Reflected field path.
        path: String,
    },
    /// An alpha cutoff is not finite or outside `0..=1`.
    #[error("material {material} has invalid alpha cutoff {cutoff}")]
    InvalidAlphaCutoff {
        /// Material containing the cutoff.
        material: MaterialId,
        /// Invalid cutoff.
        cutoff: f32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_materials_that_reference_missing_shaders() {
        let mut document = CharmeDocument::new("Invalid");
        let material = MaterialInstance::new("Body", ShaderId::new());
        let material_id = material.id();
        document.materials.push(material);

        assert!(matches!(
            document.validate(),
            Err(DocumentValidationError::MissingShader { material, .. })
                if material == material_id
        ));
    }

    #[test]
    fn slot_with_id_preserves_binding_identity() {
        let id = MaterialSlotId::new();
        let material = MaterialId::new();
        let slot = MaterialSlot::with_id(id, 2, "身体", "Body", Some(material));

        assert_eq!(slot.id(), id);
        assert_eq!(slot.source_index(), 2);
        assert_eq!(slot.material(), Some(material));
    }

    #[test]
    fn archive_character_sources_round_trip_the_selected_entry() {
        let source = CharacterSource::pmx_with_archive_entry(
            ResourcePath::project_relative("models/character.zip").unwrap(),
            "Model/character.pmx",
        );
        let encoded = ron::to_string(&source).unwrap();
        let decoded: CharacterSource = ron::from_str(&encoded).unwrap();

        assert_eq!(decoded, source);
        assert_eq!(decoded.archive_entry(), Some("Model/character.pmx"));

        let legacy: CharacterSource =
            ron::from_str(r#"(path: ProjectRelative("models/character.pmx"), format: Pmx)"#)
                .unwrap();
        assert_eq!(legacy.archive_entry(), None);
    }

    #[test]
    fn rejects_non_finite_parameter_values() {
        let mut document = CharmeDocument::new("Invalid");
        let shader =
            ShaderSource::new("Toon", ResourcePath::project_relative("toon.wgsl").unwrap());
        let mut material = MaterialInstance::new("Body", shader.id());
        material
            .parameters
            .insert("rim.strength".to_owned(), ParameterValue::F32(f32::NAN));
        document.shaders.push(shader);
        document.materials.push(material);

        assert!(matches!(
            document.validate(),
            Err(DocumentValidationError::NonFiniteParameter { path, .. })
                if path == "rim.strength"
        ));
    }
}
