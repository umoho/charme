use std::collections::BTreeMap;

use charme_core::{CharmeDocument, MaterialSlotId, ParameterValue};

/// Complete parameter projection for one renderer material slot.
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialPreviewUpdate {
    /// Stable imported slot to update.
    pub slot_id: MaterialSlotId,
    /// Complete override set for the slot's currently bound material.
    pub parameters: BTreeMap<String, ParameterValue>,
}

/// Incrementally projects the authoritative document into renderer updates.
#[derive(Debug, Default)]
pub struct PreviewSynchronizer {
    projected: BTreeMap<MaterialSlotId, BTreeMap<String, ParameterValue>>,
}

impl PreviewSynchronizer {
    /// Forgets the previous renderer projection, for example after replacing a scene.
    pub fn reset(&mut self) {
        self.projected.clear();
    }

    /// Returns complete slot updates whose document projection changed.
    pub fn synchronize(&mut self, document: &CharmeDocument) -> Vec<MaterialPreviewUpdate> {
        let next = document
            .material_slots()
            .iter()
            .map(|slot| {
                let parameters = slot
                    .material()
                    .and_then(|material| document.material(material))
                    .map(|material| material.parameters().clone())
                    .unwrap_or_default();
                (slot.id(), parameters)
            })
            .collect::<BTreeMap<_, _>>();

        let updates = next
            .iter()
            .filter(|(slot_id, parameters)| self.projected.get(slot_id) != Some(parameters))
            .map(|(slot_id, parameters)| MaterialPreviewUpdate {
                slot_id: *slot_id,
                parameters: parameters.clone(),
            })
            .collect();
        self.projected = next;
        updates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charme_core::{
        EditorCommand, EditorSession, MaterialInstance, MaterialSlot, ResourcePath, ShaderSource,
    };

    #[test]
    fn emits_only_changed_complete_slot_projections() {
        let mut session = EditorSession::new("Preview");
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

        let mut synchronizer = PreviewSynchronizer::default();
        assert_eq!(synchronizer.synchronize(session.document()).len(), 1);
        assert!(synchronizer.synchronize(session.document()).is_empty());

        session
            .apply(EditorCommand::SetMaterialParameter {
                material: material_id,
                path: "material.roughness".to_owned(),
                value: Some(ParameterValue::F32(0.8)),
            })
            .unwrap();
        let updates = synchronizer.synchronize(session.document());
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].slot_id, slot_id);
        assert_eq!(updates[0].parameters.len(), 1);

        session.undo().unwrap();
        let updates = synchronizer.synchronize(session.document());
        assert_eq!(updates.len(), 1);
        assert!(updates[0].parameters.is_empty());
    }

    #[test]
    fn reset_forces_a_complete_reprojection() {
        let document = CharmeDocument::new("Preview");
        let mut synchronizer = PreviewSynchronizer::default();
        assert!(synchronizer.synchronize(&document).is_empty());
        synchronizer.reset();
        assert!(synchronizer.synchronize(&document).is_empty());
    }
}
