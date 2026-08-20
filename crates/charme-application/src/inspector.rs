use charme_core::{CharmeDocument, MaterialId, MaterialSlotId};

use crate::{ParameterControlSpec, ShaderInspection, controls_for_material};

/// Semantic target selected in the editor hierarchy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionTarget {
    /// The complete scene.
    Scene,
    /// The imported character model.
    Model,
    /// One imported PMX material slot.
    MaterialSlot(MaterialSlotId),
    /// One standalone Charme material instance.
    Material(MaterialId),
}

/// Resolved material selection used by Inspector providers and render commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaterialSelectionContext {
    /// Original semantic hierarchy target.
    pub target: SelectionTarget,
    /// Selected PMX material slot, if the target resolves to one.
    pub slot: Option<MaterialSlotId>,
    /// Selected Charme material instance, if one is bound or directly selected.
    pub material: Option<MaterialId>,
}

impl MaterialSelectionContext {
    /// Resolves a semantic target against the current document.
    pub fn resolve(document: &CharmeDocument, target: SelectionTarget) -> Self {
        let (slot, material) = match target {
            SelectionTarget::MaterialSlot(slot_id) => {
                let material = document
                    .material_slot(slot_id)
                    .and_then(|slot| slot.material());
                (Some(slot_id), material)
            }
            SelectionTarget::Material(material_id) => {
                (None, document.material(material_id).map(|_| material_id))
            }
            SelectionTarget::Scene | SelectionTarget::Model => (None, None),
        };
        Self {
            target,
            slot,
            material,
        }
    }
}

/// Presentation row produced by an Inspector provider.
#[derive(Clone, Debug, PartialEq)]
pub enum InspectorRow {
    /// Read-only key/value text.
    Text {
        /// Stable row key.
        key: String,
        /// User-facing label.
        label: String,
        /// User-facing value.
        value: String,
    },
    /// One reflected, editable material parameter.
    Parameter(ParameterControlSpec),
    /// One reflected texture binding.
    Texture {
        /// Reflected resource path.
        path: String,
        /// Resolved resource description, when bound.
        value: Option<String>,
    },
}

/// A presentation section produced by an Inspector provider.
#[derive(Clone, Debug, PartialEq)]
pub struct InspectorSection {
    /// Stable provider key.
    pub key: &'static str,
    /// User-facing section title.
    pub title: String,
    /// Presentation rows in display order.
    pub rows: Vec<InspectorRow>,
}

impl InspectorSection {
    /// Returns true when this section contains at least one row.
    pub fn has_content(&self) -> bool {
        !self.rows.is_empty()
    }
}

/// Complete Inspector presentation for one selection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InspectorModel {
    /// Ordered sections produced by registered providers.
    pub sections: Vec<InspectorSection>,
}

impl InspectorModel {
    /// Finds one section by its stable provider key.
    pub fn section(&self, key: &str) -> Option<&InspectorSection> {
        self.sections.iter().find(|section| section.key == key)
    }
}

/// Extensible source of one Inspector section.
pub trait InspectorProvider {
    /// Builds a section for the current selection, or returns `None` when it does not apply.
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        inspection: Option<&ShaderInspection>,
    ) -> Option<InspectorSection>;
}

/// Ordered registry of platform-independent Inspector providers.
#[derive(Default)]
pub struct InspectorRegistry {
    providers: Vec<Box<dyn InspectorProvider>>,
}

impl std::fmt::Debug for InspectorRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InspectorRegistry")
            .field("provider_count", &self.providers.len())
            .finish()
    }
}

impl InspectorRegistry {
    /// Creates the standard material Inspector registry.
    pub fn standard() -> Self {
        Self {
            providers: vec![
                Box::new(MaterialSlotInspectorProvider),
                Box::new(MaterialInstanceInspectorProvider),
                Box::new(ShaderParameterProvider),
                Box::new(MaterialRenderStateProvider),
                Box::new(TextureBindingProvider),
            ],
        }
    }

    /// Adds a provider at the end of the display order.
    pub fn register(&mut self, provider: impl InspectorProvider + 'static) {
        self.providers.push(Box::new(provider));
    }

    /// Builds a complete presentation from all applicable providers.
    pub fn build(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        inspection: Option<&ShaderInspection>,
    ) -> InspectorModel {
        InspectorModel {
            sections: self
                .providers
                .iter()
                .filter_map(|provider| provider.provide(document, context, inspection))
                .collect(),
        }
    }
}

/// Provider for PMX source slot information.
#[derive(Debug, Default)]
pub struct MaterialSlotInspectorProvider;

impl InspectorProvider for MaterialSlotInspectorProvider {
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        _: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        let slot = document.material_slot(context.slot?)?;
        Some(InspectorSection {
            key: "material-source",
            title: "Source".to_owned(),
            rows: vec![
                InspectorRow::Text {
                    key: "source-index".to_owned(),
                    label: "Index".to_owned(),
                    value: slot.source_index().to_string(),
                },
                InspectorRow::Text {
                    key: "source-name".to_owned(),
                    label: "Name".to_owned(),
                    value: slot.source_name().to_owned(),
                },
                InspectorRow::Text {
                    key: "source-english-name".to_owned(),
                    label: "English Name".to_owned(),
                    value: slot.source_english_name().to_owned(),
                },
            ],
        })
    }
}

/// Provider for the selected material instance.
#[derive(Debug, Default)]
pub struct MaterialInstanceInspectorProvider;

impl InspectorProvider for MaterialInstanceInspectorProvider {
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        _: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        let material = document.material(context.material?)?;
        Some(InspectorSection {
            key: "material-instance",
            title: material.name().to_owned(),
            rows: vec![InspectorRow::Text {
                key: "shader".to_owned(),
                label: "Shader".to_owned(),
                value: material.shader().to_string(),
            }],
        })
    }
}

/// Provider for reflected Shader parameters.
#[derive(Debug, Default)]
pub struct ShaderParameterProvider;

impl InspectorProvider for ShaderParameterProvider {
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        inspection: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        let material = document.material(context.material?)?;
        let inspection = inspection?;
        Some(InspectorSection {
            key: "shader-parameters",
            title: "Parameters".to_owned(),
            rows: controls_for_material(inspection, material.parameters())
                .into_iter()
                .map(InspectorRow::Parameter)
                .collect(),
        })
    }
}

/// Provider for rasterization and blending state.
#[derive(Debug, Default)]
pub struct MaterialRenderStateProvider;

impl InspectorProvider for MaterialRenderStateProvider {
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        _: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        let material = document.material(context.material?)?;
        let state = material.render_state();
        Some(InspectorSection {
            key: "render-state",
            title: "Render State".to_owned(),
            rows: vec![
                InspectorRow::Text {
                    key: "alpha-mode".to_owned(),
                    label: "Alpha Mode".to_owned(),
                    value: format!("{:?}", state.alpha_mode),
                },
                InspectorRow::Text {
                    key: "double-sided".to_owned(),
                    label: "Double Sided".to_owned(),
                    value: state.double_sided.to_string(),
                },
            ],
        })
    }
}

/// Provider for reflected texture bindings.
#[derive(Debug, Default)]
pub struct TextureBindingProvider;

impl InspectorProvider for TextureBindingProvider {
    fn provide(
        &self,
        document: &CharmeDocument,
        context: MaterialSelectionContext,
        _: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        let material = document.material(context.material?)?;
        Some(InspectorSection {
            key: "texture-bindings",
            title: "Textures".to_owned(),
            rows: material
                .textures()
                .iter()
                .map(|(path, texture)| InspectorRow::Texture {
                    path: path.clone(),
                    value: Some(format!("{texture:?}")),
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charme_core::{
        EditorCommand, EditorSession, MaterialInstance, MaterialSlot, ResourcePath, ShaderSource,
    };

    fn selected_material() -> (EditorSession, MaterialSelectionContext) {
        let mut session = EditorSession::new("Selection");
        let shader = ShaderSource::new(
            "Preview",
            ResourcePath::project_relative("preview.wgsl").unwrap(),
        );
        let material = MaterialInstance::new("Body", shader.id());
        let material_id = material.id();
        let slot = MaterialSlot::new(0, "Body", "Body");
        let slot_id = slot.id();
        session
            .apply(EditorCommand::Transaction(vec![
                EditorCommand::UpsertShader(shader),
                EditorCommand::UpsertMaterial(material),
                EditorCommand::ReplaceMaterialSlots(vec![slot]),
                EditorCommand::BindMaterial {
                    slot: slot_id,
                    material: Some(material_id),
                },
            ]))
            .unwrap();
        let context = MaterialSelectionContext::resolve(
            session.document(),
            SelectionTarget::MaterialSlot(slot_id),
        );
        (session, context)
    }

    #[test]
    fn slot_selection_resolves_its_bound_material_id() {
        let (session, context) = selected_material();
        assert!(
            session
                .document()
                .material(context.material.unwrap())
                .is_some()
        );
        assert!(context.slot.is_some());
    }

    #[test]
    fn registry_builds_ordered_complete_sections() {
        let (session, context) = selected_material();
        let model = InspectorRegistry::standard().build(session.document(), context, None);

        assert_eq!(model.sections[0].key, "material-source");
        assert!(model.section("material-instance").unwrap().has_content());
        assert!(model.section("render-state").unwrap().has_content());
    }
}
