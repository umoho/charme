use charme_core::{CharmeDocument, MaterialId, MaterialSlotId};

use crate::ShaderInspection;

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

/// A presentation section produced by an Inspector provider.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InspectorSection {
    /// Stable provider key.
    pub key: &'static str,
    /// User-facing section title.
    pub title: String,
    /// Whether the section has editable or informative content.
    pub has_content: bool,
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
        let slot_id = context.slot?;
        let slot = document.material_slot(slot_id)?;
        Some(InspectorSection {
            key: "material-source",
            title: "Source".to_owned(),
            has_content: !slot.source_name().is_empty() || !slot.source_english_name().is_empty(),
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
        let material_id = context.material?;
        let material = document.material(material_id)?;
        Some(InspectorSection {
            key: "material-instance",
            title: material.name().to_owned(),
            has_content: true,
        })
    }
}

/// Provider for reflected Shader parameters.
#[derive(Debug, Default)]
pub struct ShaderParameterProvider;

impl InspectorProvider for ShaderParameterProvider {
    fn provide(
        &self,
        _: &CharmeDocument,
        context: MaterialSelectionContext,
        inspection: Option<&ShaderInspection>,
    ) -> Option<InspectorSection> {
        context.material?;
        let inspection = inspection?;
        Some(InspectorSection {
            key: "shader-parameters",
            title: "Parameters".to_owned(),
            has_content: !inspection.controls.is_empty(),
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
        let _material = document.material(context.material?)?;
        Some(InspectorSection {
            key: "render-state",
            title: "Render State".to_owned(),
            has_content: true,
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
            has_content: !material.textures().is_empty(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charme_core::{
        EditorCommand, EditorSession, MaterialInstance, MaterialSlot, ResourcePath, ShaderSource,
    };

    #[test]
    fn slot_selection_resolves_its_bound_material_id() {
        let mut session = EditorSession::new("Selection");
        let shader = ShaderSource::new(
            "Preview",
            ResourcePath::project_relative("preview.wgsl").unwrap(),
        );
        let material = MaterialInstance::new("Body", shader.id());
        let material_id = material.id();
        let slot = MaterialSlot::new(0, "Body", "Body");
        let slot_id = slot.id();
        session.apply(EditorCommand::UpsertShader(shader)).unwrap();
        session
            .apply(EditorCommand::UpsertMaterial(material))
            .unwrap();
        session
            .apply(EditorCommand::ReplaceMaterialSlots(vec![slot]))
            .unwrap();
        session
            .apply(EditorCommand::BindMaterial {
                slot: slot_id,
                material: Some(material_id),
            })
            .unwrap();

        let context = MaterialSelectionContext::resolve(
            session.document(),
            SelectionTarget::MaterialSlot(slot_id),
        );
        assert_eq!(context.slot, Some(slot_id));
        assert_eq!(context.material, Some(material_id));
    }
}
