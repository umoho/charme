use charme_core::{
    CharmeDocument, EditorCommand, MaterialInstance, MaterialSlot, ResourcePath, ResourcePathError,
    ShaderSource,
};
use charme_renderer::PmxSceneInfo;

/// Builds one atomic document command that reconciles imported PMX slots with
/// Charme preview materials while preserving stable existing bindings.
pub fn reconcile_pmx_materials(
    document: &CharmeDocument,
    scene: &PmxSceneInfo,
) -> Result<EditorCommand, ResourcePathError> {
    let shader_path = ResourcePath::project_relative("assets/shaders/preview_material.wgsl")?;
    let existing_shader = document
        .shaders()
        .iter()
        .find(|shader| shader.path() == &shader_path)
        .map(ShaderSource::id);
    let existing_materials = scene
        .material_slots()
        .iter()
        .map(|slot| {
            document
                .material_slot(slot.id())
                .and_then(MaterialSlot::material)
                .filter(|material| document.material(*material).is_some())
        })
        .collect::<Vec<_>>();

    let mut commands = Vec::new();
    let shader_id = existing_shader.unwrap_or_else(|| {
        let shader = ShaderSource::new("Preview Material", shader_path);
        let id = shader.id();
        commands.push(EditorCommand::UpsertShader(shader));
        id
    });
    let material_ids = scene
        .material_slots()
        .iter()
        .zip(existing_materials)
        .map(|(slot, existing)| {
            existing.unwrap_or_else(|| {
                let material = MaterialInstance::new(slot.name(), shader_id);
                let id = material.id();
                commands.push(EditorCommand::UpsertMaterial(material));
                id
            })
        })
        .collect::<Vec<_>>();
    let slots = scene
        .material_slots()
        .iter()
        .zip(material_ids)
        .map(|(slot, material)| {
            MaterialSlot::with_id(
                slot.id(),
                slot.index() as u32,
                slot.name(),
                slot.english_name(),
                Some(material),
            )
        })
        .collect();
    commands.push(EditorCommand::ReplaceMaterialSlots(slots));
    Ok(EditorCommand::Transaction(commands))
}
